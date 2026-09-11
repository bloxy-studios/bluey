//! `fingerprints` — the CLI behind `bun run fingerprints:*`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context};
use bluey_fingerprints::proxy::{self, ProxyConfig, Recorded};
use bluey_fingerprints::{har, store};
use bluey_protocols::fingerprints::{diff, DiffReport, Provider};

const USAGE: &str = "\
fingerprints — capture the official clients' request fingerprints and diff them (ADR 0009)

  fingerprints capture    <provider> [--port 1456] [--upstream URL] [--out DIR]
      Local proxy the official CLI is pointed at; every exchange is scrubbed and written
      to tests/fixtures/fingerprints/<provider>/captures/ (git-ignored). Ctrl-C stops it.
  fingerprints import-har <provider> <file.har> [--all-hosts] [--out DIR]
      The same captures from a MITM proxy's HAR export (Antigravity has no base-URL knob).
  fingerprints diff       <provider> [--capture FILE] [--endpoint NAME]
                                     [--against documented|golden|FILE] [--json]
      Newest capture vs the documented fingerprint and the blessed golden, header by
      header and field by field. Exit 1 on drift.
  fingerprints bless      <provider> [FILE]
      Promote a reviewed capture to golden/<endpoint>.json.
  fingerprints list       <provider>

providers: chatgpt (codex) · claude · antigravity (google)
runbook:   docs/PROVIDER_ACCOUNTS.md › Re-capture runbook";

struct Args {
    command: String,
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

fn parse_args(raw: Vec<String>) -> Args {
    let mut args = Args {
        command: String::new(),
        positional: Vec::new(),
        flags: HashMap::new(),
    };
    let mut iter = raw.into_iter().peekable();
    while let Some(arg) = iter.next() {
        if let Some(flag) = arg.strip_prefix("--") {
            if let Some((k, v)) = flag.split_once('=') {
                args.flags.insert(k.to_string(), v.to_string());
            } else if iter.peek().is_some_and(|n| !n.starts_with("--")) && !is_boolean_flag(flag) {
                args.flags
                    .insert(flag.to_string(), iter.next().unwrap_or_default());
            } else {
                args.flags.insert(flag.to_string(), "true".to_string());
            }
        } else if args.command.is_empty() {
            args.command = arg;
        } else {
            args.positional.push(arg);
        }
    }
    args
}

fn is_boolean_flag(flag: &str) -> bool {
    matches!(flag, "json" | "all-hosts" | "help")
}

#[tokio::main]
async fn main() -> ExitCode {
    bluey_fingerprints::ensure_crypto_provider();
    match run().await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> anyhow::Result<ExitCode> {
    let args = parse_args(std::env::args().skip(1).collect());
    if args.flags.contains_key("help") {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    match args.command.as_str() {
        "capture" => capture(&args).await,
        "import-har" => import_har(&args),
        "diff" => diff_command(&args),
        "bless" => bless(&args),
        "list" => list(&args),
        "" | "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

fn provider_arg(args: &Args) -> anyhow::Result<Provider> {
    let text = args
        .positional
        .first()
        .ok_or_else(|| anyhow!("which provider? chatgpt | claude | antigravity\n\n{USAGE}"))?;
    Provider::parse(text)
        .ok_or_else(|| anyhow!("unknown provider `{text}` — use chatgpt, claude or antigravity"))
}

fn out_root(args: &Args) -> anyhow::Result<PathBuf> {
    match args.flags.get("out") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => store::fixtures_root(),
    }
}

fn client_hint(provider: Provider, port: u16) -> String {
    match provider {
        Provider::Claude => format!("ANTHROPIC_BASE_URL=http://127.0.0.1:{port} claude"),
        Provider::Chatgpt => format!(
            "~/.codex/config.toml → chatgpt_base_url = \"http://127.0.0.1:{port}/backend-api/\"\n    (VERIFY on the day — docs/PROVIDER_ACCOUNTS.md › ChatGPT › Capture knob)"
        ),
        Provider::Antigravity => "the Antigravity app has no base-URL knob: run a MITM proxy with its CA trusted (HTTPS_PROXY),\n    export a HAR, then `bun run fingerprints:import-har antigravity <file.har>`".to_string(),
    }
}

fn describe(recorded: &Recorded) -> String {
    let c = &recorded.capture;
    let status = c
        .response
        .as_ref()
        .map(|r| r.status.to_string())
        .unwrap_or_else(|| "no response".to_string());
    let duration = c
        .response
        .as_ref()
        .and_then(|r| r.duration_ms)
        .map(|ms| format!(" {ms} ms"))
        .unwrap_or_default();
    format!(
        "  ✓ {} {} → {status}{duration} · {} · scrubbed: {}",
        c.request.method,
        c.request.path(),
        recorded
            .path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default(),
        if c.scrubbed.is_empty() {
            "nothing".to_string()
        } else {
            c.scrubbed.join(", ")
        }
    )
}

async fn capture(args: &Args) -> anyhow::Result<ExitCode> {
    let provider = provider_arg(args)?;
    let root = out_root(args)?;
    let port: u16 = match args.flags.get("port") {
        Some(p) => p.parse().context("--port must be a number")?,
        None => 1456,
    };
    let mut config = ProxyConfig::new(provider, root.clone());
    config.bind = SocketAddr::from(([127, 0, 0, 1], port));
    if let Some(upstream) = args.flags.get("upstream") {
        config.upstream = upstream.clone();
    }
    let upstream = config.upstream.clone();
    let mut handle = proxy::start(config).await?;
    println!("fingerprints capture — {}", provider.display_name());
    println!("  listening on http://{} → {upstream}", handle.addr);
    println!(
        "  captures → {} (git-ignored; scrubbed before writing)",
        store::captures_dir(&root, provider).display()
    );
    println!("  point the official client at it:");
    println!("    {}", client_hint(provider, handle.addr.port()));
    println!(
        "  Ctrl-C to stop, then `bun run fingerprints:diff {}`.",
        provider.id()
    );
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = handle.events.recv() => match event {
                Some(recorded) => println!("{}", describe(&recorded)),
                None => break,
            }
        }
    }
    handle.shutdown().await;
    println!("stopped.");
    Ok(ExitCode::SUCCESS)
}

fn import_har(args: &Args) -> anyhow::Result<ExitCode> {
    let provider = provider_arg(args)?;
    let file = args
        .positional
        .get(1)
        .ok_or_else(|| anyhow!("which HAR file? fingerprints import-har <provider> <file.har>"))?;
    let root = out_root(args)?;
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
    let captures = har::import(&text, provider, args.flags.contains_key("all-hosts"))?;
    if captures.is_empty() {
        println!(
            "no entries for {} hosts ({}) in {file} — pass --all-hosts to import everything",
            provider.display_name(),
            provider.rules().hosts.join(", ")
        );
        return Ok(ExitCode::from(1));
    }
    for capture in &captures {
        let path = store::write_capture(&root, capture)?;
        println!(
            "  ✓ {} {} → {} · {}",
            capture.request.method,
            capture.request.path(),
            capture
                .response
                .as_ref()
                .map(|r| r.status.to_string())
                .unwrap_or_else(|| "no response".into()),
            path.display()
        );
    }
    println!("{} capture(s) imported.", captures.len());
    Ok(ExitCode::SUCCESS)
}

fn diff_command(args: &Args) -> anyhow::Result<ExitCode> {
    let provider = provider_arg(args)?;
    let rules = provider.rules();
    let root = out_root(args)?;
    let endpoint_filter = args.flags.get("endpoint").map(String::as_str);
    let json = args.flags.contains_key("json");

    let actual_path = match args.flags.get("capture") {
        Some(file) => PathBuf::from(file),
        None => match store::latest_capture(&root, provider, endpoint_filter)? {
            Some(path) => path,
            None => bail!(
                "no capture for {} under {} — run `bun run fingerprints:capture {}` (or import-har) first, or pass --capture FILE",
                provider.display_name(),
                store::captures_dir(&root, provider).display(),
                provider.id()
            ),
        },
    };
    let actual = store::read_capture(&actual_path)?;
    if actual.provider != provider.id() {
        bail!(
            "{} is a {} capture, not {}",
            actual_path.display(),
            actual.provider,
            provider.id()
        );
    }
    let endpoint = store::endpoint_name(&actual);
    let actual_label = actual_path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let against = args
        .flags
        .get("against")
        .map(String::as_str)
        .unwrap_or("all");

    let mut reports: Vec<DiffReport> = Vec::new();
    if matches!(against, "all" | "documented") {
        match provider
            .documented()
            .into_iter()
            .find(|c| store::endpoint_name(c) == endpoint)
        {
            Some(documented) => reports.push(diff(
                rules,
                &documented,
                &actual,
                &format!(
                    "documented {} ({})",
                    rules.info.version, rules.info.captured_on
                ),
                &actual_label,
            )),
            None => eprintln!(
                "  (no documented capture for {} {} — endpoint `{endpoint}`)",
                actual.request.method,
                actual.request.path()
            ),
        }
    }
    if matches!(against, "all" | "golden") {
        let golden_path = store::golden_path(&root, provider, endpoint);
        if golden_path.is_file() {
            let golden = store::read_capture(&golden_path)?;
            reports.push(diff(
                rules,
                &golden,
                &actual,
                &format!("golden ({})", golden.captured_at),
                &actual_label,
            ));
        } else if against == "golden" {
            bail!("no golden capture at {}", golden_path.display());
        }
    }
    if !matches!(against, "all" | "documented" | "golden") {
        let other = store::read_capture(Path::new(against))?;
        reports.push(diff(rules, &other, &actual, against, &actual_label));
    }
    if reports.is_empty() {
        bail!("nothing to compare {} against", actual_path.display());
    }

    let drift = reports.iter().any(DiffReport::has_drift);
    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        for report in &reports {
            println!("{}", report.render());
        }
        if drift {
            println!(
                "drift → update the shaper module, bump VERSION / CAPTURED_ON, refresh the tables in docs/PROVIDER_ACCOUNTS.md, then `bun run fingerprints:bless {}` (runbook steps 6–9).",
                provider.id()
            );
        } else {
            println!("no drift — the documented fingerprint still describes this client.");
        }
    }
    Ok(if drift {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn bless(args: &Args) -> anyhow::Result<ExitCode> {
    let provider = provider_arg(args)?;
    let root = out_root(args)?;
    let from = match args.positional.get(1) {
        Some(file) => PathBuf::from(file),
        None => store::latest_capture(&root, provider, None)?.ok_or_else(|| {
            anyhow!(
                "no capture to bless — run `bun run fingerprints:capture {}` first",
                provider.id()
            )
        })?,
    };
    let path = store::bless(&root, provider, &from)?;
    println!("blessed {} → {}", from.display(), path.display());
    println!(
        "now bump VERSION / CAPTURED_ON in bluey_protocols::fingerprints::{}, refresh docs/PROVIDER_ACCOUNTS.md and regenerate the documented fixtures (UPDATE_FIXTURES=1 cargo test -p bluey-protocols fingerprints).",
        match provider {
            Provider::Chatgpt => "codex",
            Provider::Claude => "claude_code",
            Provider::Antigravity => "antigravity",
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn list(args: &Args) -> anyhow::Result<ExitCode> {
    let provider = provider_arg(args)?;
    let root = out_root(args)?;
    let files = store::list_captures(&root, provider)?;
    if files.is_empty() {
        println!(
            "no captures under {}",
            store::captures_dir(&root, provider).display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    for path in files {
        let capture = store::read_capture(&path)?;
        println!(
            "  {}  {} {}  {}  {}",
            path.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            capture.request.method,
            capture.request.path(),
            capture
                .response
                .as_ref()
                .map(|r| r.status.to_string())
                .unwrap_or_else(|| "—".into()),
            capture.client.as_deref().unwrap_or("unknown client")
        );
    }
    let golden = store::golden_dir(&root, provider);
    if golden.is_dir() {
        println!("golden: {}", golden.display());
    }
    Ok(ExitCode::SUCCESS)
}
