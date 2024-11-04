use std::env::{self, VarError};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::ExitCode;
use std::time::Duration;
use std::{process, thread};

use chrono::Utc;
use env_logger;
use env_logger::Env;
use eyre::{bail, eyre};
use feed_parrot::feed::ParsedFeed;
use feed_parrot::mastodon::models::{MastodonState, Visibility};
use feed_parrot::models::{Service, Services};
use feed_parrot::{db, mastodon};
use getopts::Options;
use log::{debug, error, info, warn};
use redb::{Database, WriteTransaction};
use reqwest::blocking::Client;
use url::Url;

use feed_parrot::crawler::ConditionalRequest;
use feed_parrot::crawler::{self, FeedData};
use feed_parrot::mastodon::Mastodon;
use feed_parrot::social_network::{
    AccessMode, Posted, Registration, SocialNetwork, ValidationResult,
};
#[cfg(feature = "twitter")]
use feed_parrot::twitter::Twitter;
use feed_parrot::Delay;

const LOG_ENV_VAR: &str = "FEED_PARROT_LOG";
const DATABASE_ENV_VAR: &str = "FEED_PARROT_DATABASE";
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Copy, Clone)]
struct FeedParrot<'a> {
    access_mode: AccessMode,
    cond_req: ConditionalRequest,
    delay: Delay,
    post_visibility: Visibility,
    services: &'a Services,
    feed_urls: &'a [Url],
}

fn main() -> ExitCode {
    match simple_eyre::install() {
        Ok(()) => (),
        Err(report) => {
            eprintln!("Unable to initialise error reporter: {:?}", report);
            return ExitCode::FAILURE;
        }
    };

    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(report) => {
            eprintln!("Error: {:?}", report);
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> eyre::Result<()> {
    if let Err(env::VarError::NotPresent) = env::var(LOG_ENV_VAR) {
        env::set_var(LOG_ENV_VAR, "info");
    }

    let env = Env::new().filter(LOG_ENV_VAR);
    env_logger::init_from_env(env);

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("d", "database", "path to database file", "FILE");
    opts.optflag("h", "help", "print this help information");
    opts.optopt("l", "list", "list items in db", "TYPE");
    opts.optflag("n", "dryrun", "don't post statuses or update the db");
    opts.optflag("r", "register", "register with a service (requires -s)");
    opts.optmulti("s", "service", "filter action by service", "SERVICE");
    opts.optopt("u", "url-file", "read feed URLs from FILE", "FILE");
    opts.optopt(
        "",
        "visibility",
        "post visibility (when service supports it)",
        "VISIBILITY",
    );
    opts.optopt(
        "w",
        "wait",
        "time to wait between posting new items",
        "DURATION",
    );
    opts.optflag("", "no-cache", "ignore stored cache headers");
    let matches = opts.parse(&args[1..])?;

    if matches.opt_present("h") {
        print_usage(&program, &opts);
        return Ok(());
    }
    let access_mode = if matches.opt_present("n") {
        AccessMode::ReadOnly
    } else {
        AccessMode::ReadWrite
    };

    let delay = matches
        .opt_get("w")?
        .unwrap_or_else(|| Delay::from_secs(60));

    let db_path = match env::var(DATABASE_ENV_VAR) {
        Ok(path) => Some(path),
        Err(VarError::NotPresent) => matches.opt_str("d").map(|v| v.to_string()),
        Err(err) => {
            // FIXME: Return error
            eprintln!(
                "Error reading {} environment variable: {err}",
                DATABASE_ENV_VAR
            );
            process::exit(1);
        }
    };
    let Some(db_path) = db_path else {
        // FIXME: Return error
        eprintln!(
            "Database path must be supplied with -d or {} environment variable",
            DATABASE_ENV_VAR
        );
        process::exit(1);
    };

    let db = db::establish_connection(&db_path)?;

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(TIMEOUT)
        // .read_timeout(TIMEOUT)
        .timeout(Duration::from_secs(2 * 60))
        .user_agent(format!("Feed Parrot/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    if let Some(type_) = matches.opt_str("l") {
        return list_items(&db, client.clone(), &type_);
    }

    let services = matches
        .opt_strs("s")
        .into_iter()
        .map(|s| s.parse())
        .collect::<Result<Vec<Service>, _>>()?;

    let post_visibility = matches
        .opt_get("visibility")?
        .unwrap_or_else(|| Visibility::Unlisted);

    if matches.opt_present("r") {
        if access_mode == AccessMode::ReadOnly {
            bail!("registration cannot be run in dry-run mode");
        }
        register(&db, client.clone(), &services)
    } else {
        let services = if services.is_empty() {
            Services::All
        } else {
            Services::Specific(services)
        };

        let mut urls = matches
            .free
            .iter()
            .map(|url| Url::parse(url))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(path) = matches.opt_str("u") {
            let file = BufReader::new(File::open(&path)?);
            for line in file.lines() {
                let line = line?;
                let url = Url::parse(&line)?;
                urls.push(url);
            }
        };
        urls.sort();
        urls.dedup();

        let cond_req = if matches.opt_present("no-cache") {
            ConditionalRequest::Disabled
        } else {
            ConditionalRequest::Enabled
        };
        let settings = FeedParrot {
            access_mode,
            cond_req,
            delay,
            post_visibility,
            services: &services,
            feed_urls: &urls,
        };
        run(&db, client.clone(), settings)
    }
}

fn print_usage(program: &str, opts: &Options) {
    let brief = format!("Usage: {} [options] URL", program);
    eprint!("{}", opts.usage(&brief));
}

fn list_items(db: &Database, client: Client, type_: &str) -> eyre::Result<()> {
    match type_ {
        "feeds" => list_feeds(db),
        "services" => list_services(db, &client),
        _ => Err(eyre!(
            "unknown item type: {type_}. Valid types: feeds, services"
        )),
    }
}

fn list_feeds(db: &Database) -> eyre::Result<()> {
    let feeds = db::load_feeds(db)?;
    feeds.iter().for_each(|feed| println!("{}", feed.url));
    Ok(())
}

fn list_services(db: &Database, client: &Client) -> eyre::Result<()> {
    let services = db::load_services(db, &Services::All)?;
    for service_data in services.iter() {
        match service_data.service {
            Service::Mastodon => {
                let state: MastodonState = rmp_serde::from_slice(&service_data.data)?;
                let instance = state.instance.clone();
                let mastodon = Mastodon {
                    access_mode: AccessMode::ReadOnly,
                    post_visibility: Visibility::Unlisted,
                    state,
                };
                let account = mastodon.verify_credentials(client)?;

                println!(
                    "{}: @{} on {}",
                    service_data.service, account.username, instance
                );
            }
            Service::Twitter => todo!(),
        }
    }
    Ok(())
}

fn run(db: &Database, client: Client, settings: FeedParrot<'_>) -> eyre::Result<()> {
    // This is a hack to compile the valid URL regex as early as possible
    // because it's quite slow on low-powered devices like Raspberry Pi
    // Zero. By kicking this off in a background thread it can happen in
    // parallel with feed fetching.
    thread::spawn(|| mastodon::precompile_regex());

    let services = db::load_services(db, settings.services)?
        .into_iter()
        .map(|service_data| {
            // Turn the service data into SocialMedia trait objects
            match service_data.service {
                Service::Mastodon => {
                    let state: MastodonState = rmp_serde::from_slice(&service_data.data)?;
                    Ok(Box::from(Mastodon {
                        access_mode: settings.access_mode,
                        post_visibility: settings.post_visibility,
                        state,
                    }) as Box<dyn SocialNetwork>)
                }
                Service::Twitter => todo!(),
            }
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if services.is_empty() {
        bail!("no configured services to post to")
    }

    // For each feed, fetch it and pass new entries to announce new posts for each enabled service
    if settings.feed_urls.is_empty() {
        bail!("no feeds URLs supplied")
    }

    for feed_url in settings.feed_urls {
        // Load the feed from the db
        let mut feed = match db::load_feed(&db, &feed_url) {
            Ok(feed) => feed,
            Err(err) => {
                // TODO: make this exit non-zero
                error!("Unable to load feed from database {feed_url}: {err}");
                continue;
            }
        };

        let res = crawler::refresh_feed(client.clone(), settings.cond_req, &mut feed);

        let feed_data = match res {
            Ok(feed) => feed,
            Err(err) => {
                error!("Unable to fetch {feed_url}: {err}");
                // TODO: make this exit non-zero
                continue;
            }
        };

        match feed_data {
            FeedData::NotModified => {
                info!("{feed_url}: No new posts");
            }
            FeedData::Updated(parsed_feed) => {
                thread::scope(|scope| {
                    let mut threads = Vec::with_capacity(services.len());
                    for network in services.iter() {
                        if feed.had_initial_sync {
                            let client = client.clone();
                            let handle = scope.spawn(|| {
                                announce_new_posts(
                                    db,
                                    client,
                                    network.as_ref(),
                                    &parsed_feed,
                                    settings.delay,
                                )
                            });
                            threads.push((feed_url, handle));
                        } else {
                            info!("Performing initial sync of {feed_url}");
                            match perform_initial_sync(db, network.as_ref(), &parsed_feed) {
                                Ok(()) => feed.had_initial_sync = true,
                                Err(report) => {
                                    // TODO: make this exit non-zero
                                    error!(
                                        "Failed to complete initial sync of {feed_url}: {:?}",
                                        report
                                    );
                                }
                            }
                        }
                    }
                    for (feed_url, thread) in threads {
                        // NOTE(unwrap): join returns Err when the thread panicked to we propagate that
                        // panic.
                        match thread.join().unwrap() {
                            Ok(()) => (),
                            Err(report) => {
                                // TODO: make this exit non-zero
                                error!("Failed to announce new posts for {feed_url}: {:?}", report);
                            }
                        }
                    }
                })
            }
        }

        // Persist the feed (with updated cache headers) now that processing was successful
        if settings.access_mode == AccessMode::ReadWrite {
            let tx = db.begin_write()?;
            db::save_feed(&tx, &feed)?;
            tx.commit()?;
        }
    }

    Ok(())
}

fn perform_initial_sync(
    db: &Database,
    network: &dyn SocialNetwork,
    feed: &ParsedFeed,
) -> eyre::Result<()> {
    if !network.is_writeable() {
        return Ok(());
    }

    let guids = feed.items().map(|item| item.guid);
    let now = Utc::now();
    db::mark_items_seen(db, network.service(), now, guids)?;
    Ok(())
}

fn announce_new_posts(
    db: &Database,
    client: Client,
    network: &dyn SocialNetwork,
    feed: &ParsedFeed,
    delay: Delay,
) -> eyre::Result<()> {
    let mut new_items = Vec::new();
    for item in feed.items() {
        // Determine if this item has been posted before
        if db::item_posted(db, network.service(), &item.guid)? {
            debug!("skip {}, already posted", item.guid);
            continue;
        }

        if item.is_future_post() {
            debug!("skip future dated post {}", item.guid);
            continue;
        }

        new_items.push(item);
    }

    // Sort new items so they are published oldest to newest
    new_items.sort_by_key(|item| item.date_published);

    // Publish them!
    let mut item_iter = new_items.iter().peekable();
    while let Some(item) = item_iter.next() {
        info!(
            "New post to announce: [{}] {}",
            item.guid,
            item.title.as_deref().unwrap_or("<empty>")
        );

        let status = network.prepare_post(item)?;
        let ready_post = match status.validate(&db, network.service()) {
            ValidationResult::Ok(ok) => ok,
            ValidationResult::Duplicate(_post) => {
                warn!("skipping duplicate post [{}]", item.guid);
                continue;
            }
            ValidationResult::Error(err) => {
                error!("unable to publish post [{}]: {:?}", item.guid, err);
                continue;
            }
        };

        let tx = db.begin_write()?;
        let res: eyre::Result<_> = {
            let posted = network.publish_post(&client, ready_post)?;
            mark_post_published(&tx, network, posted)?;
            Ok(())
        };

        match res {
            Ok(()) => tx.commit()?,
            Err(err) => {
                // FIXME: Is there a better way to rollback?
                drop(tx);
                error!("Unable to publish post [{}]: {:?}", item.guid, err);
            }
        };

        if network.is_writeable() && item_iter.peek().is_some() {
            debug!("waiting before sending next post");
            thread::sleep(delay.duration());
        }
    }

    Ok(())
}

fn register(db: &Database, client: Client, services: &[Service]) -> eyre::Result<()> {
    let service = match services {
        [service] => service,
        _ => {
            bail!("exactly one service must be supplied with -s to register")
        }
    };

    match service {
        Service::Twitter => {
            // Twitter::register()?
            todo!()
        }
        Service::Mastodon => Mastodon::register(db, client),
    }
}

fn mark_post_published(
    tx: &WriteTransaction,
    network: &dyn SocialNetwork,
    post: Posted,
) -> eyre::Result<()> {
    if network.is_writeable() {
        db::mark_post_tooted(tx, network.service(), post)?;
    }

    Ok(())
}

mod null_twitter {
    use eyre::bail;
    use feed_parrot::feed::NewFeedItem;
    use feed_parrot::social_network::{Posted, PotentialPost, ReadyPost};
    use feed_parrot::{
        models::Service,
        social_network::{Registration, SocialNetwork},
    };

    use reqwest::blocking::Client;

    pub struct Twitter;

    impl Registration for Twitter {
        fn register(_db: &redb::Database, _client: Client) -> eyre::Result<()> {
            bail!("Twitter support is not enabled")
        }
    }

    impl SocialNetwork for Twitter {
        fn service(&self) -> Service {
            Service::Twitter
        }

        fn is_writeable(&self) -> bool {
            false
        }

        fn prepare_post(&self, item: &NewFeedItem) -> eyre::Result<PotentialPost> {
            bail!("Twitter support is not enabled")
        }

        fn publish_post(&self, _client: &Client, _post: ReadyPost) -> eyre::Result<Posted> {
            bail!("Twitter support is not enabled")
        }
    }
}
