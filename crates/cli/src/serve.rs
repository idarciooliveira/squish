//! `squish serve`: the web UI and the HTTP API on localhost (ADR-0014, ADR-0018, ADR-0019).

use std::ffi::OsString;
use std::num::NonZeroUsize;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "squish serve", version, about = "Start the web UI and the HTTP API")]
pub struct ServeCli {
    /// Address to bind. Use 0.0.0.0 to accept other machines
    #[arg(long, default_value = "127.0.0.1", env = "SQUISH_HOST")]
    host: String,

    /// Port to bind. 0 picks a free one
    #[arg(short, long, default_value_t = 8080, env = "SQUISH_PORT")]
    port: u16,

    /// Open the page in the default browser
    #[arg(long)]
    open: bool,

    /// Concurrent compressions [default: core count]
    #[arg(short, long, value_name = "N")]
    jobs: Option<NonZeroUsize>,

    /// Largest accepted upload, in MB
    #[arg(long, value_name = "MB", default_value = "25", value_parser = parse_megabytes)]
    max_body: usize,

    /// Refuse images above this many megapixels
    #[arg(long, value_name = "MP", default_value = "50", value_parser = crate::parse_megapixels)]
    max_pixels: u64,

    /// Give up on one compression after this many seconds
    #[arg(long, value_name = "SECS", default_value = "30", value_parser = parse_seconds)]
    timeout: Duration,
}

/// MB as the API reports it: `25` is 26214400 bytes (ADR-0018).
fn parse_megabytes(s: &str) -> Result<usize, String> {
    match s.trim().parse::<f64>() {
        Ok(mb) if mb.is_finite() && mb > 0.0 && mb * 1048576.0 < usize::MAX as f64 => {
            Ok((mb * 1048576.0).round() as usize)
        }
        _ => Err(format!("invalid size {s:?}, expected a positive number of MB such as 25 or 0.5")),
    }
}

fn parse_seconds(s: &str) -> Result<Duration, String> {
    match s.trim().parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs > 0.0 && secs < 1e9 => Ok(Duration::from_secs_f64(secs)),
        _ => Err(format!("invalid timeout {s:?}, expected a positive number of seconds")),
    }
}

/// `args` starts at `serve`, which clap reads as the program name.
pub fn main(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let cli = ServeCli::parse_from(args);
    start(cli)
}

#[cfg(feature = "server")]
fn start(cli: ServeCli) -> ExitCode {
    let defaults = squish_server::Config::default();
    let target = format!("{}:{}", cli.host, cli.port);
    let config = squish_server::Config {
        host: cli.host,
        port: cli.port,
        jobs: cli.jobs.map_or(defaults.jobs, NonZeroUsize::get),
        max_body: cli.max_body,
        max_pixels: cli.max_pixels,
        timeout: cli.timeout,
    };
    let open = cli.open;
    let result = squish_server::run(config, |addr| {
        // A wildcard address can't be browsed to. Point the browser at loopback instead.
        let browse = if addr.ip().is_unspecified() {
            let loopback: std::net::IpAddr = if addr.is_ipv4() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                std::net::Ipv6Addr::LOCALHOST.into()
            };
            std::net::SocketAddr::new(loopback, addr.port())
        } else {
            addr
        };
        let url = format!("http://{browse}");
        println!("squish listening on http://{addr}");
        if open && let Err(e) = open_browser(&url) {
            eprintln!("squish: could not open a browser ({e}), visit {url}");
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("squish: could not serve on {target}: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "server"))]
fn start(_cli: ServeCli) -> ExitCode {
    eprintln!("squish: this build has no web server; rebuild with the `server` feature");
    ExitCode::from(crate::USAGE)
}

#[cfg(feature = "server")]
fn open_browser(url: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    command.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        ServeCli::command().debug_assert();
    }

    #[test]
    fn limits_parse() {
        assert_eq!(parse_megabytes("25"), Ok(26_214_400));
        assert_eq!(parse_megabytes("0.5"), Ok(524_288));
        assert!(parse_megabytes("0").is_err());
        assert!(parse_megabytes("1e300").is_err());
        assert_eq!(parse_seconds("1.5"), Ok(Duration::from_millis(1500)));
        assert!(parse_seconds("-1").is_err());
        assert!(parse_seconds("inf").is_err());
    }

    #[test]
    fn defaults_follow_the_adr() {
        let cli = ServeCli::try_parse_from(["serve"]).unwrap();
        assert_eq!(cli.max_body, 25 * 1024 * 1024);
        assert_eq!(cli.max_pixels, 50_000_000);
        assert_eq!(cli.timeout, Duration::from_secs(30));
        assert!(!cli.open && cli.jobs.is_none());
    }
}
