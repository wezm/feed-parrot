use env_logger;
use feed_parrot::db;
use feed_parrot::models::{Service, Services};
use redb::Database;
use url::Url;

use std::env::{self, VarError};
use std::error::Error;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;
use std::{process, thread};

// use diesel::{Connection, PgConnection};
// use dotenv::dotenv;
use env_logger::Env;
use getopts::Options;
use log::{debug, error, info};

use feed_parrot::mastodon::Mastodon;
use feed_parrot::social_network::{AccessMode, SocialNetwork};
#[cfg(twitter)]
use feed_parrot::twitter::Twitter;
use feed_parrot::Delay;
#[cfg(not(feature = "twitter"))]
use null_twitter::Twitter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const LOG_ENV_VAR: &str = "FEED_PARROT_LOG";
const DATABASE_ENV_VAR: &str = "FEED_PARROT_DATABASE";
const ONE_SECOND: Duration = Duration::from_secs(1);
const SLEEP_TIME: usize = 600; // 10 minutes

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<(), Box<dyn Error>> {
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
    opts.optflag("h", "help", "print this help information");
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
    dbg!(wait);

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

    let services = matches
        .opt_strs("s")
        .into_iter()
        .map(|s| s.parse())
        .collect::<Result<Vec<Service>, _>>()?;

    if matches.opt_present("r") {
        register(&db, access_mode, &services)
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
        run(&db, access_mode, &services, &urls)
    }
}

fn print_usage(program: &str, opts: &Options) {
    let brief = format!("Usage: {} [options] URL", program);
    eprint!("{}", opts.usage(&brief));
}

fn run(
    db: &Database,
    access_mode: AccessMode,
    services: &Services,
    feed_urls: &[Url],
) -> Result<(), Box<dyn Error>> {
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

    let services = db::load_services(db, services)?;
    if services.is_empty() {
        return Err("no configured services to post to".into());
    }

    // For each feed, fetch it and pass new entries to announce new posts for each enabled service

    // announce_new_posts(db, network, )

    Ok(())
}

fn announce_new_posts<S: SocialNetwork>(
    db: &Database,
    network: &S,
    // categories: &Categories,
) -> Result<(), Box<dyn Error>> {
    // for post in <S as SocialNetwork>::unpublished_posts(conn)? {
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
    // }
    //
    // Ok(())
    todo!()
}

fn register(
    db: &Database,
    access_mode: AccessMode,
    services: &[Service],
) -> Result<(), Box<dyn Error>> {
    let service = match services {
        [service] => service,
        _ => {
            return Err("Exactly one service must be supplied with -s to register".into());
        }
    };

    // match service {
    //     Service::Twitter => Twitter::register(),
    //     Service::Mastodon => Mastodon::register(),
    // }

    Ok(())
}

mod null_twitter {
    use feed_parrot::social_network::SocialNetwork;

    pub struct Twitter;

    impl SocialNetwork for Twitter {
        fn from_env(
            _access_mode: feed_parrot::social_network::AccessMode,
        ) -> Result<Self, Box<dyn std::error::Error>> {
            Ok(Twitter)
        }

        fn register() -> Result<(), Box<dyn std::error::Error>> {
            Err(String::from("Twitter support is not enabled").into())
        }

        // fn unpublished_posts(
        //     _connection: &diesel::PgConnection,
        // ) -> diesel::QueryResult<Vec<feed_parrot::models::Post>> {
        //     unimplemented!("Twitter support is not enabled")
        // }

        fn publish_post(
            &self,
            _post: &feed_parrot::models::Post,
            _categories: &[std::rc::Rc<feed_parrot::categories::Category>],
        ) -> Result<(), Box<dyn std::error::Error>> {
            Err(String::from("Twitter support is not enabled").into())
        }

        // fn mark_post_published(
        //     &self,
        //     _connection: &diesel::PgConnection,
        //     _post: feed_parrot::models::Post,
        // ) -> diesel::QueryResult<()> {
        //     unimplemented!("Twitter support is not enabled")
        // }
    }
}
