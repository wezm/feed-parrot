use std::env::{self, VarError};
use std::process;
use std::process::ExitCode;
use std::time::Duration;

use env_logger;
use env_logger::Env;
use eyre::{bail, Context};
use feed_parrot::feed::ParsedFeed;
use feed_parrot::mastodon::models::MastodonState;
use feed_parrot::models::{Service, Services};
use feed_parrot::{db, mastodon};
use getopts::Options;
use log::{debug, error, info};
use redb::Database;
use reqwest::Client;
use url::Url;

use feed_parrot::crawler::{self, FeedData};
use feed_parrot::crawler::{ConditionalRequest, SyncType};
use feed_parrot::mastodon::Mastodon;
use feed_parrot::social_network::{AccessMode, Registration, SocialNetwork};
#[cfg(twitter)]
use feed_parrot::twitter::Twitter;
use feed_parrot::Delay;

const LOG_ENV_VAR: &str = "FEED_PARROT_LOG";
const DATABASE_ENV_VAR: &str = "FEED_PARROT_DATABASE";
const ONE_SECOND: Duration = Duration::from_secs(1);
const SLEEP_TIME: usize = 600; // 10 minutes
const TIMEOUT: Duration = Duration::from_secs(30);

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
    opts.optflag("n", "dryrun", "don't post statuses or update the db");
    opts.optmulti("s", "service", "filter action by service", "SERVICE");
    opts.optopt(
        "w",
        "wait",
        "time to wait between posting new items",
        "DURATION",
    );
    opts.optflag("r", "register", "register with a service (requires -s)");
    opts.optopt("i", "instance", "instance to register to", "URL");
    opts.optflag("h", "help", "print this help information");
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

    let wait = matches
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

    let client = reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .timeout(Duration::from_secs(2 * 60))
        .user_agent(format!("Feed Parrot/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let services = matches
        .opt_strs("s")
        .into_iter()
        .map(|s| s.parse())
        .collect::<Result<Vec<Service>, _>>()?;

    if matches.opt_present("r") {
        if access_mode == AccessMode::ReadOnly {
            bail!("registration cannot be run in dry-run mode");
        }

        let instance: Option<Url> = matches
            .opt_get("i")
            .wrap_err("unable to parse instance URL")?;
        register(&db, client.clone(), access_mode, instance, &services)
    } else {
        let services = if services.is_empty() {
            Services::All
        } else {
            Services::Specific(services)
        };
        let urls = matches
            .free
            .iter()
            .map(|url| Url::parse(url))
            .collect::<Result<Vec<_>, _>>()?;
        let cond_req = if matches.opt_present("no-cache") {
            ConditionalRequest::Disabled
        } else {
            ConditionalRequest::Enabled
        };
        run(&db, client.clone(), access_mode, cond_req, &services, &urls)
    }
}

fn print_usage(program: &str, opts: &Options) {
    let brief = format!("Usage: {} [options] URL", program);
    eprint!("{}", opts.usage(&brief));
}

fn run(
    db: &Database,
    client: Client,
    access_mode: AccessMode,
    cond_req: ConditionalRequest,
    services: &Services,
    feed_urls: &[Url],
) -> eyre::Result<()> {
    // let database_url = env_var("DATABASE_URL")?;
    // let conn = db::establish_connection(&database_url)?;
    // info!("Connected to database, access_mode: {:?}", access_mode);
    //
    // let categories = Categories::load();
    //
    // // create twiter and masto clients, with appropriate access mode
    // let twitter = Twitter::from_env(access_mode)?;
    // let mastodon = Mastodon::from_env(access_mode)?;
    //
    // debug!("Entering main loop");
    // let term = Arc::new(AtomicBool::new(false));
    // signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))?;
    // signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term))?;
    //
    // while !term.load(Ordering::Relaxed) {
    //     if toot {
    //         debug!("Checking for new posts to toot");
    //         if let Err(err) = announce_new_posts(&mastodon, &conn, &categories) {
    //             // TODO: Log Sentry error
    //             error!("Error tooting new posts: {}", err);
    //         }
    //     }
    //     if term.load(Ordering::Relaxed) {
    //         break;
    //     }
    //     if tweet {
    //         debug!("Checking for new posts to tweet");
    //         if let Err(err) = announce_new_posts(&twitter, &conn, &categories) {
    //             error!("Error tweeting new posts: {}", err);
    //         }
    //     }
    //
    //     if !doloop {
    //         break;
    //     }
    //     for _ in 0..SLEEP_TIME {
    //         if term.load(Ordering::Relaxed) {
    //             break;
    //         }
    //         thread::sleep(ONE_SECOND);
    //     }
    // }
    //

    let services = db::load_services(db, services)?
        .into_iter()
        .map(|service_data| {
            // Turn the service data into SocialMedia trait objects
            match service_data.service {
                Service::Mastodon => {
                    let state: MastodonState = rmp_serde::from_slice(&service_data.data)?;
                    Ok(Box::from(Mastodon { access_mode, state }) as Box<dyn SocialNetwork>)
                }
                Service::Twitter => todo!(),
            }
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if services.is_empty() {
        bail!("no configured services to post to")
    }

    // For each feed, fetch it and pass new entries to announce new posts for each enabled service
    if feed_urls.is_empty() {
        bail!("no feeds URLs supplied")
    }

    // No point spinning up 12 or 24 threads for this little program, cap at four
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(4)
                .min(4),
        )
        .thread_name("feed-parrot")
        .enable_all()
        .build()?;

    let sync_type = SyncType::Initial;

    for feed_url in feed_urls {
        // Load the feed from the db
        let mut feed = match db::load_feed(&db, &feed_url) {
            Ok(feed) => feed,
            Err(err) => {
                error!("Unable to load feed from database {feed_url}: {err}");
                continue;
            }
        };

        let res = runtime.block_on(crawler::refresh_feed(
            client.clone(),
            sync_type,
            cond_req,
            &mut feed,
        ));

        let parsed_feed = match res {
            Ok(feed) => feed,
            Err(err) => {
                error!("Unable to fetch {feed_url}: {err}");
                continue;
            }
        };

        match parsed_feed {
            FeedData::NotModified => {
                info!("{feed_url}: No new posts");
            }
            FeedData::Updated(feed) => {
                for network in services.iter() {
                    match announce_new_posts(db, network.as_ref(), &feed) {
                        Ok(()) => (),
                        Err(report) => {
                            error!("Failed to announce new posts for {feed_url}: {:?}", report);
                        }
                    }
                }
            }
        }

        // Persist the feed (with updated cache headers) now that processing was successful
        if access_mode == AccessMode::ReadWrite {
            let tx = db.begin_write()?;
            db::save_feed(&tx, &feed)?;
            tx.commit()?;
        }
    }

    Ok(())
}

fn announce_new_posts(
    db: &Database,
    network: &dyn SocialNetwork,
    feed: &ParsedFeed,
) -> eyre::Result<()> {
    for item in feed.items() {
        // Determine if this item has been posted before
        if db::item_posted(db, network.service(), &item.guid)? {
            debug!("skip {}, already posted", item.guid)
        }

        info!(
            "New post to announce: [{}] {}",
            item.guid,
            item.title.as_deref().unwrap_or("<empty>")
        );

        let tx = db.begin_write()?;
        {
            if let Err(err) = network.publish_post(&tx, &item) {
                error!("Unable to announce post [{}]: {:?}", item.guid, err);
            }
        }
        tx.commit()?;

        //     let post_id = post.id;
        //     info!("New post to announce: [{}] {}", post_id, post.title);
        //     let toot_result = db::post_categories(conn, &post, categories)
        //         .map_err(|err| err.into())
        //         .and_then(|post_categories| {
        //             conn.transaction::<_, Box<dyn Error>, _>(|| {
        //                 network.publish_post(&post, &post_categories)?;
        //                 network.mark_post_published(conn, post)?;
        //
        //                 Ok(())
        //             })
        //         });
        //
        //     if let Err(err) = toot_result {
        //         error!("Unable to announce post [{}]: {}", post_id, err);
        //     }

    }

    Ok(())
}

fn register(
    db: &Database,
    client: Client,
    access_mode: AccessMode,
    instance: Option<Url>,
    services: &[Service],
) -> eyre::Result<()> {
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
        Service::Mastodon => {
            let Some(instance) = instance else {
                bail!("instance must be specified with -i to register with Mastodon")
            };
            // let masto = Mastodon {
            //     access_mode,
            //     instance,
            // };
            let instance = mastodon::Instance(instance);
            instance.register(db, client)
        }
    }
}

mod null_twitter {
    use eyre::bail;
    use feed_parrot::{
        models::Service,
        social_network::{Registration, SocialNetwork},
    };

    pub struct Twitter;

    impl Registration for Twitter {
        fn register(&self, _db: &redb::Database, _client: reqwest::Client) -> eyre::Result<()> {
            bail!("Twitter support is not enabled")
        }
    }

    impl SocialNetwork for Twitter {
        fn service(&self) -> Service {
            Service::Twitter
        }

        fn publish_post(
            &self,
            _tx: &redb::WriteTransaction,
            _item: &feed_parrot::feed::NewFeedItem,
        ) -> eyre::Result<()> {
            bail!("Twitter support is not enabled")
        }
    }
}
