<h1 align="center">
  🦜<br>
  Feed Parrot
</h1>

<div align="center">
  <strong>Post entries from an RSS feed to Mastodon. Runs on BSD, Linux, macOS, Windows, and
  more.</strong>
</div>

<br>

<div align="center">
  <a href="https://cirrus-ci.com/github/wezm/feed-parrot">
    <img src="https://api.cirrus-ci.com/github/wezm/feed-parrot.svg" alt="Build Status"></a>
  <a href="https://crates.io/crates/feed-parrot">
    <img src="https://img.shields.io/crates/v/feed-parrot.svg" alt="Version">
  </a>
  <img src="https://img.shields.io/crates/l/feed-parrot.svg" alt="License">
</div>

<br>


TODO: Write longer description here.

TODO: Include rationale/motivation/goals

Install
-------

### Pre-compiled Binary

Pre-compiled binaries are available for a number of platforms:

* FreeBSD 13+ amd64
* Linux x86\_64
* Linux aarch64
* MacOS Universal
* Windows x86\_64

Check the [latest release] for download links.

### From Source

See [Build From Source](#build-from-source) below.

<!--

### Package Manager

`feed-parrot` is packaged in these package managers:

* Arch Linux: `feed-parrot`
* Brew: `brew install feed-parrot`
* Chimera Linux: `feed-parrot`

-->

Usage
-----

### Overview

```
USAGE:
    feed-parrot [OPTIONS] [FEED_URL]...

ARGS:
    <URL>...
        One or more feed URLs to check for new items.

OPTIONS:
    -h, --help
            Prints help information

    -n, --dry-run
            Don't clone the repository but print what would be done.

    -V, --version
            Prints version information

ENVIRONMENT
    FEED_PARROT_LOG
        Set log level and filter.
```

### Initial Setup

Feed Parrot stores its state in a database. The path to the database is
specified with `-d`, which is required. You will need to authenticate with each
service that you want to post to. Pass `-r` and the service name to register
with that service. The supported services are currently:

- `mastodon`

E.g. `feed-parrot -r mastodon -d feedparrot.db`

### Posting New Items

Once you have registered with the services to post to, run Feed Parrot with one
or more feed URLs to check. Items that exist on the initial fetch of each URL
will not be posted.

E.g. `feed-parrot -d feedparrot.db https://example.com/feed`

### Delay Between Posts

By default Feed Parrot posts new items with a 1-minute delay between each item.
This can be controlled by the `-w` (wait) argument. The delay may be specified
in seconds or minutes using an `s` or `m` suffix. E.g. `30s` or `10m`.

<!--
### Only Posting Some Services

-s service
-->

### Dry Run

Running Feed Parrot with `-n` will cause it to print what will be posted but
won't actually post anything. E.g.

```
$ feed-parrot -d feedparrot.db -n
[TODO sample output]
```

### Logging

Logging is controlled with the `FEED_PARROT_LOG` environment variable. The log
levels from least verbose to most verbose are:

* `off` (no logs)
* `error`
* `warn`
* `info`
* `debug`
* `trace`

The default log level is `info`. To change the log level to `debug` use
`FEED_PARROT_LOG=debug`. The `FEED_PARROT_LOG` variable also supports
filtering. For example to only show `trace` messages from `feedlynx` (and not
some of the libraries it uses) you would specify:
`FEED_PARROT_LOG=trace=feedlynx`. For more details refer to the [env_logger
documentation][env_logger].


Build from Source
-----------------

**Minimum Supported Rust Version:** 1.79.0

Feed Parrot is implemented in Rust. See the Rust website for [instructions on
installing the toolchain][rustup].

### From Git Checkout or Release Tarball

Build the binary with:

    cargo build --release --locked

The binary will be in `target/release/feed-parrot`.

### From crates.io

    cargo install feed-parrot

### Compile-time Options (Cargo Features)

Feed Parrot supports the following compile-time options:

* `rust-tls` (default): use the `rust-tls` crate for handling TLS connections.
* `native-tls`: use the `native-tls` crate for handling TLS connections. This
  might be a better option when building on Windows.

To build with `native-tls` invoke Cargo as follows:

    cargo build --release --locked --no-default-features --features native-tls

If packaging Feed Parrot for an operating system registry it might make sense
to use `native-tls`. On Linux and BSD systems that adds a dependency on
OpenSSL.

### Man Page

A man page is available. Building it requires the [scdoc] tool:

    make -C doc/Makefile

The man page is written to `doc/feed-parrot.1`.

#### Installation

There is an `install` target for installing the man page. `PREFIX`, `MANDIR`,
and `DESTDIR` are honoured if supplied.

    make -C doc/Makefile install

<!--

Credits
-------

TODO

Licence
-------

This project is dual licenced under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/wezm/feed-parrot/blob/master/LICENSE-APACHE))
- MIT license ([LICENSE-MIT](https://github.com/wezm/feed-parrot/blob/master/LICENSE-MIT))

at your option.

-->

[rustup]: https://www.rust-lang.org/tools/install
[env_logger]: https://docs.rs/env_logger/0.11.3/env_logger/index.html
[scdoc]: https://git.sr.ht/~sircmpwn/scdoc
