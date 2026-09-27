//! `tower service …` (D§10, scheduled-jobs decision §"Always-on server"):
//! run `tower serve` as a per-user service — a launchd agent on macOS, a
//! systemd user unit on Linux — so scheduled jobs fire without an open
//! terminal. The unit carries the caller's `TOWER_HOME` and `PATH` (herdr and
//! the harnesses must be findable); the API token stays in its file.

use std::path::{Path, PathBuf};
use std::process::Output;

use anyhow::{bail, Context};
use clap::Subcommand;
use tokio::process::Command;
use tower_client::Client;

const DEFAULT_LABEL: &str = "dev.tower.serve";

#[derive(Debug, Subcommand)]
pub enum ServiceCmd {
    /// Install `tower serve` as a user service and start it (launchd / systemd)
    Install {
        /// Print the rendered plist / unit to stdout and change nothing
        #[arg(long)]
        print: bool,
        /// Service label (unit name); use a distinct one to run a second
        /// instance, e.g. with another TOWER_HOME
        #[arg(long, default_value = DEFAULT_LABEL)]
        label: String,
    },
    /// Stop and remove the user service (no-op if not installed)
    Uninstall {
        /// Service label (unit name)
        #[arg(long, default_value = DEFAULT_LABEL)]
        label: String,
    },
    /// Show unit file, service manager state, and server reachability
    Status {
        /// Service label (unit name)
        #[arg(long, default_value = DEFAULT_LABEL)]
        label: String,
    },
}

pub async fn run(cmd: ServiceCmd) -> anyhow::Result<()> {
    let platform = Platform::detect()?;
    match cmd {
        ServiceCmd::Install { print, label } => install(platform, &label, print).await,
        ServiceCmd::Uninstall { label } => uninstall(platform, &label).await,
        ServiceCmd::Status { label } => status(platform, &label).await,
    }
}

#[derive(Debug, Clone, Copy)]
enum Platform {
    Launchd,
    Systemd,
}

impl Platform {
    fn detect() -> anyhow::Result<Self> {
        match std::env::consts::OS {
            "macos" => Ok(Self::Launchd),
            "linux" => Ok(Self::Systemd),
            other => bail!(
                "`tower service` supports macOS (launchd) and Linux (systemd); \
                 this is {other} — run `tower serve` under your own supervisor"
            ),
        }
    }

    /// Where the plist / unit file for `label` lives.
    fn unit_file(self, label: &str) -> anyhow::Result<PathBuf> {
        let dirs = directories::BaseDirs::new().context("cannot determine home directory")?;
        Ok(match self {
            Self::Launchd => dirs
                .home_dir()
                .join("Library/LaunchAgents")
                .join(format!("{label}.plist")),
            // honors XDG_CONFIG_HOME, which systemd searches too
            Self::Systemd => dirs
                .config_dir()
                .join("systemd/user")
                .join(format!("{label}.service")),
        })
    }
}

/// The caller's environment the service must inherit.
struct ServiceEnv {
    exe: PathBuf,
    tower_home: Option<String>,
    path: String,
}

impl ServiceEnv {
    fn capture() -> anyhow::Result<Self> {
        let exe = std::env::current_exe().context("cannot locate the tower executable")?;
        let exe = exe
            .canonicalize()
            .with_context(|| format!("canonicalizing {}", exe.display()))?;
        // launchd/systemd start in `/`, so a relative TOWER_HOME must be pinned
        let tower_home = match std::env::var_os("TOWER_HOME") {
            Some(home) => {
                let home = PathBuf::from(home);
                let home = if home.is_absolute() {
                    home
                } else {
                    std::env::current_dir()
                        .context("resolving relative TOWER_HOME")?
                        .join(home)
                };
                Some(
                    home.into_os_string()
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("TOWER_HOME is not valid UTF-8"))?,
                )
            }
            None => None,
        };
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
        Ok(Self {
            exe,
            tower_home,
            path,
        })
    }

    /// launchd log directory: `$TOWER_HOME/logs`, else `~/Library/Logs/tower`.
    fn launchd_log_dir(&self) -> anyhow::Result<PathBuf> {
        Ok(match &self.tower_home {
            Some(home) => Path::new(home).join("logs"),
            None => directories::BaseDirs::new()
                .context("cannot determine home directory")?
                .home_dir()
                .join("Library/Logs/tower"),
        })
    }
}

async fn install(platform: Platform, label: &str, print: bool) -> anyhow::Result<()> {
    let env = ServiceEnv::capture()?;
    let tower_home = env.tower_home.as_deref();
    let (rendered, log_dir) = match platform {
        Platform::Launchd => {
            let log_dir = env.launchd_log_dir()?;
            let plist = launchd_plist(label, &env.exe, tower_home, &env.path, &log_dir);
            (plist, Some(log_dir))
        }
        Platform::Systemd => (systemd_unit(&env.exe, tower_home, &env.path), None),
    };
    if print {
        print!("{rendered}");
        return Ok(());
    }

    let file = platform.unit_file(label)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&file, &rendered).with_context(|| format!("writing {}", file.display()))?;

    match platform {
        Platform::Launchd => {
            let log_dir = log_dir.expect("launchd renders a log dir");
            // launchd does not create missing parents for StandardOutPath
            std::fs::create_dir_all(&log_dir)
                .with_context(|| format!("creating {}", log_dir.display()))?;
            let domain = gui_domain().await?;
            let target = format!("{domain}/{label}");
            // reinstall: unload the old definition first
            if launchctl_loaded(&target).await? {
                cmd("launchctl", &["bootout", &target]).await?;
                wait_unloaded(&target).await?;
            }
            let plist = file.to_string_lossy();
            checked("launchctl", &["bootstrap", &domain, &plist]).await?;
            println!("installed launchd agent {label}");
            println!("  plist:  {}", file.display());
            println!("  runs:   {} serve", env.exe.display());
            if let Some(home) = tower_home {
                println!("  TOWER_HOME={home}");
            }
            println!(
                "  logs:   {}",
                log_dir.join(format!("{label}.err.log")).display()
            );
            println!(
                "          {}",
                log_dir.join(format!("{label}.out.log")).display()
            );
        }
        Platform::Systemd => {
            let unit = format!("{label}.service");
            checked("systemctl", &["--user", "daemon-reload"]).await?;
            checked("systemctl", &["--user", "enable", &unit]).await?;
            // restart (not `start`) so a reinstall picks up the new unit
            checked("systemctl", &["--user", "restart", &unit]).await?;
            println!("installed systemd user unit {unit}");
            println!("  unit:   {}", file.display());
            println!("  runs:   {} serve", env.exe.display());
            if let Some(home) = tower_home {
                println!("  TOWER_HOME={home}");
            }
            println!("  logs:   journalctl --user -u {unit}");
            println!("  to keep it running while logged out: loginctl enable-linger $USER");
        }
    }
    Ok(())
}

async fn uninstall(platform: Platform, label: &str) -> anyhow::Result<()> {
    let file = platform.unit_file(label)?;
    let file_exists = file.exists();
    match platform {
        Platform::Launchd => {
            let target = format!("{}/{label}", gui_domain().await?);
            let loaded = launchctl_loaded(&target).await?;
            if !file_exists && !loaded {
                println!("launchd agent {label} is not installed");
                return Ok(());
            }
            if loaded {
                // a failure here means it was already gone: fine
                cmd("launchctl", &["bootout", &target]).await?;
                wait_unloaded(&target).await?;
            }
            remove_if_exists(&file)?;
            println!("uninstalled launchd agent {label}");
        }
        Platform::Systemd => {
            let unit = format!("{label}.service");
            let known = systemctl_known(&unit).await?;
            if !file_exists && !known {
                println!("systemd user unit {unit} is not installed");
                return Ok(());
            }
            // ignore failure: the unit may already be stopped or unknown
            cmd("systemctl", &["--user", "disable", "--now", &unit]).await?;
            remove_if_exists(&file)?;
            checked("systemctl", &["--user", "daemon-reload"]).await?;
            println!("uninstalled systemd user unit {unit}");
        }
    }
    if file_exists {
        println!("  removed {}", file.display());
    }
    Ok(())
}

async fn status(platform: Platform, label: &str) -> anyhow::Result<()> {
    let file = platform.unit_file(label)?;
    let present = if file.exists() { "present" } else { "missing" };
    println!("{:<12} {} ({present})", "unit file", file.display());

    let service = match platform {
        Platform::Launchd => {
            let target = format!("{}/{label}", gui_domain().await?);
            let out = cmd("launchctl", &["print", &target]).await?;
            if out.status.success() {
                describe_launchctl_print(&String::from_utf8_lossy(&out.stdout))
            } else {
                "not loaded".to_string()
            }
        }
        Platform::Systemd => {
            let unit = format!("{label}.service");
            let out = cmd("systemctl", &["--user", "is-active", &unit]).await?;
            // is-active prints the state even when exiting non-zero
            let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if state.is_empty() {
                "unknown".to_string()
            } else {
                state
            }
        }
    };
    println!("{:<12} {service}", "service");

    let server = match Client::connect(None) {
        Ok(c) => c.get("/healthz").await.map(|v| v["ok"] == true),
        Err(e) => Err(e),
    };
    let server = match server {
        Ok(true) => "healthz ok".to_string(),
        Ok(false) => "healthz reports a database error".to_string(),
        Err(e) => format!("not reachable: {e:#}"),
    };
    println!("{:<12} {server}", "server");
    Ok(())
}

/// Summarize `launchctl print` output: `state = …`, `pid = …`,
/// `last exit code = …` (first occurrence; nested sections come later).
fn describe_launchctl_print(out: &str) -> String {
    let field = |key: &str| {
        out.lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(key)?.trim_start().strip_prefix('='))
            .map(|v| v.trim().to_string())
    };
    let state = field("state").unwrap_or_else(|| "unknown".into());
    let mut s = format!("loaded, {state}");
    if let Some(pid) = field("pid") {
        s.push_str(&format!(" (pid {pid})"));
    } else if let Some(code) = field("last exit code") {
        s.push_str(&format!(" (last exit code {code})"));
    }
    s
}

fn remove_if_exists(file: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", file.display())),
    }
}

/// Run a command, returning its output whatever the exit status.
async fn cmd(prog: &str, args: &[&str]) -> anyhow::Result<Output> {
    Command::new(prog)
        .args(args)
        .output()
        .await
        .with_context(|| format!("running `{prog}`"))
}

/// Run a command and fail with its stderr on a non-zero exit.
async fn checked(prog: &str, args: &[&str]) -> anyhow::Result<Output> {
    let out = cmd(prog, args).await?;
    if !out.status.success() {
        bail!(
            "`{prog} {}` failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out)
}

/// `gui/<uid>` — the per-user launchd domain (uid via `id -u`, no libc).
async fn gui_domain() -> anyhow::Result<String> {
    let out = checked("id", &["-u"]).await?;
    let uid: u32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .context("parsing `id -u` output")?;
    Ok(format!("gui/{uid}"))
}

async fn launchctl_loaded(target: &str) -> anyhow::Result<bool> {
    Ok(cmd("launchctl", &["print", target]).await?.status.success())
}

/// `bootout` returns before the job is fully gone; `bootstrap` right after
/// fails with an I/O error, so wait for it to disappear.
async fn wait_unloaded(target: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        if !launchctl_loaded(target).await? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bail!("{target} is still loaded 5s after `launchctl bootout`")
}

/// Whether systemd knows the unit at all (loaded, even if inactive).
async fn systemctl_known(unit: &str) -> anyhow::Result<bool> {
    let out = cmd(
        "systemctl",
        &["--user", "show", "--property=LoadState", "--value", unit],
    )
    .await?;
    let state = String::from_utf8_lossy(&out.stdout);
    Ok(out.status.success() && !matches!(state.trim(), "" | "not-found"))
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Render the launchd user-agent plist running `<exe> serve`.
fn launchd_plist(
    label: &str,
    exe: &Path,
    tower_home: Option<&str>,
    path: &str,
    log_dir: &Path,
) -> String {
    let x = xml_escape;
    let exe = x(&exe.to_string_lossy());
    let out_log = x(&log_dir.join(format!("{label}.out.log")).to_string_lossy());
    let err_log = x(&log_dir.join(format!("{label}.err.log")).to_string_lossy());
    let mut env = format!("\t\t<key>PATH</key>\n\t\t<string>{}</string>\n", x(path));
    if let Some(home) = tower_home {
        env.push_str(&format!(
            "\t\t<key>TOWER_HOME</key>\n\t\t<string>{}</string>\n",
            x(home)
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
		<string>serve</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
{env}	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>StandardOutPath</key>
	<string>{out_log}</string>
	<key>StandardErrorPath</key>
	<string>{err_log}</string>
</dict>
</plist>
"#,
        label = x(label),
    )
}

/// Quote a value for a systemd unit line: `\` and `"` escaped, `%`
/// specifiers doubled, and (for `ExecStart=`) `$` expansion doubled.
fn systemd_quote(s: &str, exec: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' if exec => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render the systemd user unit running `<exe> serve`.
fn systemd_unit(exe: &Path, tower_home: Option<&str>, path: &str) -> String {
    let mut env = format!(
        "Environment={}\n",
        systemd_quote(&format!("PATH={path}"), false)
    );
    if let Some(home) = tower_home {
        env.push_str(&format!(
            "Environment={}\n",
            systemd_quote(&format!("TOWER_HOME={home}"), false)
        ));
    }
    format!(
        "[Unit]\n\
         Description=tower server (tower serve)\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} serve\n\
         {env}\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = systemd_quote(&exe.to_string_lossy(), true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/opt/homebrew/bin:/usr/bin:/bin";

    fn plist(home: Option<&str>) -> String {
        launchd_plist(
            "dev.tower.serve",
            Path::new("/usr/local/bin/tower"),
            home,
            PATH,
            Path::new("/Users/me/Library/Logs/tower"),
        )
    }

    #[test]
    fn plist_runs_exe_serve_with_keepalive_and_path() {
        let p = plist(None);
        assert!(p.contains(
            "<array>\n\t\t<string>/usr/local/bin/tower</string>\n\t\t<string>serve</string>\n\t</array>"
        ));
        assert!(p.contains("<key>KeepAlive</key>\n\t<true/>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(p.contains(&format!("<key>PATH</key>\n\t\t<string>{PATH}</string>")));
        assert!(p.contains("<string>/Users/me/Library/Logs/tower/dev.tower.serve.err.log</string>"));
        assert!(p.contains("<string>/Users/me/Library/Logs/tower/dev.tower.serve.out.log</string>"));
    }

    #[test]
    fn plist_tower_home_present_iff_given() {
        assert!(!plist(None).contains("TOWER_HOME"));
        let p = plist(Some("/srv/tower"));
        assert!(p.contains("<key>TOWER_HOME</key>\n\t\t<string>/srv/tower</string>"));
    }

    #[test]
    fn plist_escapes_xml_in_values() {
        let p = launchd_plist(
            "dev.tower.serve",
            Path::new("/tmp/a&b<c/tower"),
            Some("/tmp/h&<"),
            PATH,
            Path::new("/tmp/logs"),
        );
        assert!(p.contains("<string>/tmp/a&amp;b&lt;c/tower</string>"));
        assert!(p.contains("<string>/tmp/h&amp;&lt;</string>"));
        assert!(!p.contains("a&b"));
    }

    #[test]
    fn systemd_unit_lines() {
        let u = systemd_unit(Path::new("/usr/local/bin/tower"), Some("/srv/tower"), PATH);
        assert!(u.contains("\nExecStart=\"/usr/local/bin/tower\" serve\n"));
        assert!(u.contains(&format!("\nEnvironment=\"PATH={PATH}\"\n")));
        assert!(u.contains("\nEnvironment=\"TOWER_HOME=/srv/tower\"\n"));
        assert!(u.contains("\nRestart=on-failure\n"));
        assert!(u.contains("\nWantedBy=default.target\n"));
        assert!(!systemd_unit(Path::new("/t"), None, PATH).contains("TOWER_HOME"));
    }

    #[test]
    fn systemd_quote_escapes_specifiers() {
        assert_eq!(systemd_quote("/a b/100%/$x", true), "\"/a b/100%%/$$x\"");
        assert_eq!(systemd_quote("P=a\"b\\c$", false), "\"P=a\\\"b\\\\c$\"");
    }

    #[test]
    fn launchctl_print_summary() {
        let out = "gui/501/dev.tower.serve = {\n\tactive count = 1\n\tstate = running\n\tpid = 4242\n\tendpoints = {\n\t\tstate = active\n\t}\n}";
        assert_eq!(describe_launchctl_print(out), "loaded, running (pid 4242)");
        let out = "x = {\n\tstate = not running\n\tlast exit code = 78: EX_CONFIG\n}";
        assert_eq!(
            describe_launchctl_print(out),
            "loaded, not running (last exit code 78: EX_CONFIG)"
        );
    }
}
