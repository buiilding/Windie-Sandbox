//! Repository-only Windie development supervisor.
//!
//! `windie-dev` is intentionally separate from the installed runtime CLI.
//! It owns foreground development processes, release packaging orchestration,
//! and benchmarks that should never be part of a public Windie installation.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use windie::config;
use windie::conversation::ConversationId;
use windie::gateway::GatewayUrl;
use windie::llm::{BaseUrl, ModelName};
use windie::output::TerminalOutput;
use windie::perf::{self, BenchmarkCategory, BenchmarkMode, BenchmarkOptions};

const MODEL: &str = "openai/gpt-4o-mini";

#[tokio::main]
async fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty()
        || args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "help" | "--help" | "-h"))
    {
        return print_help();
    }
    match args.as_slice() {
        [arg] if matches!(arg.as_str(), "--version" | "-V" | "-v") => {
            println!("windie-dev {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [command, action] if command == "dev" && action == "up" => dev_up().await,
        [command, action, component] if command == "dev" && action == "run" => {
            dev_run(component).await
        }
        [command, action] if command == "dev" && action == "status" => dev_status().await,
        [command, action] if command == "dev" && action == "down" => dev_down().await,
        [command, action] if command == "release" && action == "build" => {
            release_script("package-release").await
        }
        [command, action] if command == "release" && action == "install" => {
            release_script("test-local-installer").await
        }
        [command, action] if command == "release" && action == "verify" => release_verify().await,
        [command, rest @ ..] if command == "bench" => benchmark(rest).await,
        [command, subject, rest @ ..] if command == "compare" && subject == "baseline" => {
            compare_baseline(rest).await
        }
        [command, subject, rest @ ..] if command == "update" && subject == "baseline" => {
            update_baseline(rest).await
        }
        _ => {
            print_help()?;
            bail!("invalid windie-dev command")
        }
    }
}

/// Prints the development-only command surface.
fn print_help() -> Result<()> {
    println!(
        "windie-dev\n\nUsage:\n  windie-dev dev up\n  windie-dev dev run <gateway|api|inspector>\n  windie-dev dev status\n  windie-dev dev down\n  windie-dev release build\n  windie-dev release install\n  windie-dev release verify\n  windie-dev bench [conversation_id] [options]\n  windie-dev compare baseline [options]\n  windie-dev update baseline [options]\n\nDevelopment processes run in the foreground. Press Ctrl-C to stop them.\n\nBenchmark options:\n  --all --runs <n> --json\n  --persistence --conversation --serialization --runtime\n  --sessions --tools --mutations --mcp --api --lifecycle"
    );
    Ok(())
}

/// Runs gateway, API, and the hot-reloading Inspector together.
async fn dev_up() -> Result<()> {
    println!("windie-dev: starting gateway");
    let mut gateway = spawn_gateway().await?;
    if let Err(error) = wait_for_gateway(&mut gateway).await {
        stop_child(&mut gateway).await;
        let _ = stop_gateway_process().await;
        return Err(error);
    }

    let mut api = spawn_component("api")?;
    let mut inspector = spawn_component("inspector")?;
    println!("windie-dev: api and inspector are running; press Ctrl-C to stop");

    let result = supervise_children(&mut gateway, &mut api, &mut inspector).await;
    stop_child(&mut api).await;
    stop_child(&mut inspector).await;
    stop_child(&mut gateway).await;
    stop_gateway_process().await?;
    result
}

/// Runs one development component in the foreground.
async fn dev_run(component: &str) -> Result<()> {
    match component {
        "gateway" => {
            let mut gateway = spawn_gateway().await?;
            if let Err(error) = wait_for_gateway(&mut gateway).await {
                stop_child(&mut gateway).await;
                let _ = stop_gateway_process().await;
                return Err(error);
            }
            println!("windie-dev: gateway is running; press Ctrl-C to stop");
            let result = supervise_one(&mut gateway).await;
            stop_child(&mut gateway).await;
            stop_gateway_process().await?;
            result
        }
        "api" | "inspector" => {
            let mut child = spawn_component(component)?;
            println!("windie-dev: {component} is running; press Ctrl-C to stop");
            let result = supervise_one(&mut child).await;
            stop_child(&mut child).await;
            result
        }
        _ => bail!("unknown component {component}; expected gateway, api, or inspector"),
    }
}

/// Reports health for all three local runtime endpoints.
async fn dev_status() -> Result<()> {
    println!("windie-dev dev status");
    println!(
        "gateway: {}",
        health(&format!("{}/health", config::gateway_url())).await
    );
    println!(
        "api: {}",
        health(&format!("{}/api/health", config::api_url())).await
    );
    println!(
        "inspector: {}",
        health(&format!("http://{}/", config::inspector_address())).await
    );
    Ok(())
}

/// Stops all detached runtime components owned by the current environment.
async fn dev_down() -> Result<()> {
    for args in [
        ["api", "stop"].as_slice(),
        ["inspector", "stop"].as_slice(),
        ["gateway", "stop"].as_slice(),
    ] {
        run_windie(args).await?;
    }
    Ok(())
}

/// Stops any Bifrost process left by the development watcher.
async fn stop_gateway_process() -> Result<()> {
    run_windie(&["gateway", "stop"]).await
}

/// Starts Bifrost through Air so Go changes rebuild and restart the gateway.
async fn spawn_gateway() -> Result<Child> {
    let root = repository_root()?;
    let bifrost_root = root.join("vendor/bifrost");
    let transport_root = bifrost_root.join("transports/bifrost-http");
    if !transport_root.join("main.go").is_file() {
        bail!("Bifrost source is missing at {}", transport_root.display());
    }
    if !std::process::Command::new("air")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        bail!(
            "Bifrost hot reload requires Air; install it with `go install github.com/air-verse/air@latest`"
        );
    }
    prepare_bifrost_workspace(&bifrost_root).await?;

    let app_dir = windie::local::windie_home_dir()?.join("bifrost/data");
    let port = gateway_url().port();
    let air_config = write_bifrost_air_config(&root, &app_dir, &port)?;
    let mut command = Command::new("air");
    command
        .args(["-c"])
        .arg(air_config)
        .current_dir(root)
        .env("BIFROST_UI_DEV", "true")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .context("failed to start the Bifrost hot-reload process")
}

/// Writes a temporary Air configuration with a Windie-owned Bifrost binary
/// name. The name matters because the normal gateway stop path verifies that
/// the process listening on the configured port is actually Bifrost.
fn write_bifrost_air_config(repository_root: &Path, app_dir: &Path, port: &str) -> Result<PathBuf> {
    let bifrost_root = repository_root.join("vendor/bifrost");
    let config_dir = repository_root.join("target/windie-dev");
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("failed to create {}", config_dir.display()))?;
    let config_path = config_dir.join("bifrost-air.toml");
    let config = format!(
        "root = {root}\ntmp_dir = \"transports/bifrost-http/tmp\"\n\n[build]\ncmd = \"cd transports/bifrost-http && go build -tags dev -o ./tmp/bifrost-http .\"\nbin = \"transports/bifrost-http/tmp/bifrost-http\"\nargs_bin = [{host}, {host_value}, {port}, {port_value}, {app_flag}, {app_value}]\ndelay = 1000\nexclude_dir = [\"assets\", \"tmp\", \"vendor\", \"testdata\", \"ui\", \"node_modules\", \"core/tests\", \"tests\", \"docs\"]\nexclude_regex = [\"_test.go\"]\nwatch_dirs = [\"cli\", \"core\", \"framework\", \"plugins\", \"transports/bifrost-http\"]\ninclude_ext = [\"go\", \"tpl\", \"tmpl\", \"html\"]\nkill_delay = \"1s\"\nlog = \"transports/bifrost-http/tmp/build-errors.log\"\nstop_on_error = true\nsend_interrupt = true\n\n[log]\ntime = false\n\n[misc]\nclean_on_exit = false\n",
        root = toml_string(&bifrost_root),
        host = toml_string("-host"),
        host_value = toml_string("127.0.0.1"),
        port = toml_string("-port"),
        port_value = toml_string(port),
        app_flag = toml_string("-app-dir"),
        app_value = toml_string(app_dir),
    );
    fs::write(&config_path, config)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(config_path)
}

/// Encodes one path or argument as a TOML basic string.
fn toml_string(value: impl AsRef<Path>) -> String {
    format!(
        "\"{}\"",
        value
            .as_ref()
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}

/// Waits for the Air-managed Bifrost process to become healthy.
async fn wait_for_gateway(child: &mut Child) -> Result<()> {
    let health_url = format!("{}/health", config::gateway_url());
    for _ in 0..300 {
        if health(&health_url).await == "running" {
            return Ok(());
        }
        if let Some(status) = child.try_wait().context("failed to poll Bifrost process")? {
            bail!("Bifrost development process exited with {status}");
        }
        sleep(Duration::from_millis(200)).await;
    }
    bail!("Bifrost did not become healthy within 60 seconds")
}

/// Creates the local Bifrost Go workspace used by Air's local-module build.
async fn prepare_bifrost_workspace(bifrost_root: &Path) -> Result<()> {
    if !bifrost_root.join("go.work").is_file() {
        run_go(
            bifrost_root,
            &[
                "work",
                "init",
                "./cli",
                "./core",
                "./framework",
                "./transports",
            ],
        )
        .await?;
    }

    let mut modules = vec![
        "./cli".to_string(),
        "./core".to_string(),
        "./framework".to_string(),
        "./transports".to_string(),
    ];
    let plugins = bifrost_root.join("plugins");
    if plugins.is_dir() {
        for entry in fs::read_dir(plugins).context("failed to read Bifrost plugins")? {
            let path = entry?.path();
            if path.join("go.mod").is_file() {
                let name = path
                    .file_name()
                    .ok_or_else(|| anyhow!("invalid Bifrost plugin path {}", path.display()))?
                    .to_string_lossy();
                modules.push(format!("./plugins/{name}"));
            }
        }
    }
    for module in modules {
        run_go(bifrost_root, &["work", "use", module.as_str()]).await?;
    }
    run_go(bifrost_root, &["work", "sync"]).await
}

/// Runs one local Go workspace command and preserves its terminal output.
async fn run_go(directory: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("go")
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to run Go workspace command")?;
    if status.success() {
        Ok(())
    } else {
        bail!("Go workspace command exited with {status}")
    }
}

/// Starts one foreground child with inherited terminal output.
fn spawn_component(component: &str) -> Result<Child> {
    let root = repository_root()?;
    let mut command = if component == "inspector" {
        let mut command = Command::new(npm_command());
        command
            .arg("start")
            .arg("--prefix")
            .arg(root.join("dev/windie-inspector"));
        command.env("BROWSER", "none");
        command
    } else {
        cargo_component_command(component)
    };

    command
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .with_context(|| format!("failed to start development {component}"))
}

/// Uses cargo-watch when available so Rust API changes restart automatically.
fn cargo_component_command(component: &str) -> Command {
    let mut command = Command::new("cargo");
    let command_line = format!("run --bin windie -- {component} run");
    if watch_enabled() {
        command.args(["watch", "-x", command_line.as_str()]);
    } else {
        command.args(["run", "--bin", "windie", "--", component, "run"]);
    }
    command
}

/// Enables cargo-watch by default when it is installed; set `WINDIE_DEV_WATCH=0`
/// to force a single Rust process for environments without file watching.
fn watch_enabled() -> bool {
    env::var("WINDIE_DEV_WATCH")
        .map(|value| value != "0")
        .unwrap_or_else(|_| {
            std::process::Command::new("cargo")
                .args(["watch", "--version"])
                .output()
                .is_ok_and(|output| output.status.success())
        })
}

/// Waits until one foreground child exits or Ctrl-C is pressed.
async fn supervise_children(
    gateway: &mut Child,
    api: &mut Child,
    inspector: &mut Child,
) -> Result<()> {
    loop {
        if let Some(status) = gateway
            .try_wait()
            .context("failed to poll Bifrost process")?
        {
            return Err(anyhow!("Bifrost development process exited with {status}"));
        }
        if let Some(status) = api.try_wait().context("failed to poll API process")? {
            return Err(anyhow!("API development process exited with {status}"));
        }
        if let Some(status) = inspector
            .try_wait()
            .context("failed to poll Inspector process")?
        {
            return Err(anyhow!(
                "Inspector development process exited with {status}"
            ));
        }
        if ctrl_c_or_tick().await {
            return Ok(());
        }
    }
}

/// Waits until one foreground child exits or Ctrl-C is pressed.
async fn supervise_one(child: &mut Child) -> Result<()> {
    loop {
        if let Some(status) = child.try_wait().context("failed to poll process")? {
            return Err(anyhow!("development process exited with {status}"));
        }
        if ctrl_c_or_tick().await {
            return Ok(());
        }
    }
}

/// Returns true for Ctrl-C and false for the normal polling tick.
async fn ctrl_c_or_tick() -> bool {
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.is_ok(),
        _ = sleep(Duration::from_millis(250)) => false,
    }
}

/// Terminates one child and tolerates a process that already exited.
async fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill().await;
        let _ = timeout(Duration::from_secs(3), child.wait()).await;
    }
}

/// Runs the public Windie executable through Cargo so the dev command uses the
/// current checkout rather than a stale installed binary.
async fn run_windie(args: &[&str]) -> Result<()> {
    let root = repository_root()?;
    let status = Command::new("cargo")
        .args(["run", "--bin", "windie", "--"])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to run windie lifecycle command")?;
    if status.success() {
        Ok(())
    } else {
        bail!("windie lifecycle command exited with {status}")
    }
}

/// Executes one platform-native release helper from the repository root.
async fn release_script(script: &str) -> Result<()> {
    let root = repository_root()?;
    let package_args = if script == "package-release" {
        Some((
            release_target()?,
            release_asset_label()?,
            root.join("target/release-dist"),
        ))
    } else {
        None
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("powershell");
        command
            .args(["-ExecutionPolicy", "Bypass", "-File"])
            .arg(root.join(format!("scripts/{script}.ps1")));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("bash");
        command.arg(root.join(format!("scripts/{script}.sh")));
        command
    };
    if let Some((target, label, dist)) = package_args {
        command.args([target, label]).arg(dist);
    }
    let status = command.current_dir(root).status().await?;

    if status.success() {
        Ok(())
    } else {
        bail!("release command {script} exited with {status}")
    }
}

/// Returns the native Rust target used by the checked-in release scripts.
fn release_target() -> Result<String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin".to_string()),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin".to_string()),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu".to_string()),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu".to_string()),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc".to_string()),
        _ => bail!("unsupported host for native release packaging"),
    }
}

/// Returns the asset label used by the public installer for the current host.
fn release_asset_label() -> Result<String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-aarch64".to_string()),
        ("macos", "x86_64") => Ok("macos-x86_64".to_string()),
        ("linux", "aarch64") => Ok("linux-aarch64".to_string()),
        ("linux", "x86_64") => Ok("linux-x86_64".to_string()),
        ("windows", "x86_64") => Ok("windows-x86_64".to_string()),
        _ => bail!("unsupported host for release asset packaging"),
    }
}

/// Verifies that the local installer produced an executable that responds.
async fn release_verify() -> Result<()> {
    let install_dir = local_install_dir()?;
    let executable = install_dir.join(if cfg!(windows) {
        "windie.exe"
    } else {
        "windie"
    });
    if !executable.is_file() {
        bail!(
            "local Windie executable was not found at {}",
            executable.display()
        );
    }
    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .await
        .with_context(|| format!("failed to run {}", executable.display()))?;
    if !version.status.success() {
        bail!("local Windie version check failed");
    }
    print!("{}", String::from_utf8_lossy(&version.stdout));
    println!("verified: {}", executable.display());
    Ok(())
}

/// Finds the isolated local-installer binary for the current host.
fn local_install_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("WINDIE_INSTALL_DIR") {
        return Ok(PathBuf::from(path));
    }
    let root = repository_root()?;
    let label = match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => bail!("unsupported host for local release verification"),
    };
    Ok(env::var("WINDIE_LOCAL_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target/local-installer").join(label))
        .join("bin"))
}

/// Runs one provider-free benchmark command through the shared perf module.
async fn benchmark(args: &[String]) -> Result<()> {
    let (mode, conversation_id, options) = parse_benchmark_args(args)?;
    let output = TerminalOutput;
    if options.runs == 1 && !options.json {
        let baseline = perf::run(
            mode,
            conversation_id,
            gateway_url(),
            base_url(),
            ModelName::new(MODEL),
            &options.categories,
        )
        .await?;
        output.performance_baseline(&baseline);
    } else {
        let report = perf::run_report(
            mode,
            conversation_id,
            gateway_url(),
            base_url(),
            ModelName::new(MODEL),
            &options,
        )
        .await?;
        if options.json {
            output.performance_report_json(&report)?;
        } else {
            output.performance_report(&report);
        }
    }
    Ok(())
}

/// Compares the current local benchmark run with the checked-in baseline.
async fn compare_baseline(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let baseline_path = perf::default_baseline_path()?;
    let baseline = perf::read_report(&baseline_path)?;
    let current = perf::run_report(
        BenchmarkMode::Local,
        None,
        gateway_url(),
        base_url(),
        ModelName::new(MODEL),
        &options,
    )
    .await?;
    TerminalOutput.performance_comparison(&perf::compare_reports(&baseline, &current));
    Ok(())
}

/// Replaces the checked-in benchmark baseline with a current local run.
async fn update_baseline(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let baseline_path = perf::default_baseline_path()?;
    let report = perf::run_report(
        BenchmarkMode::Local,
        None,
        gateway_url(),
        base_url(),
        ModelName::new(MODEL),
        &options,
    )
    .await?;
    perf::write_report(&baseline_path, &report)?;
    TerminalOutput.updated_baseline(&baseline_path);
    Ok(())
}

/// Parses the optional conversation selector and benchmark flags.
fn parse_benchmark_args(
    args: &[String],
) -> Result<(BenchmarkMode, Option<ConversationId>, BenchmarkOptions)> {
    let (mode, conversation_id, option_args) = match args.first() {
        Some(value) if !value.starts_with('-') => (
            BenchmarkMode::Conversation,
            Some(ConversationId::new(value)),
            &args[1..],
        ),
        _ => (BenchmarkMode::Local, None, args),
    };
    Ok((mode, conversation_id, parse_options(option_args)?))
}

/// Parses benchmark options shared by run, compare, and update commands.
fn parse_options(args: &[String]) -> Result<BenchmarkOptions> {
    let mut options = BenchmarkOptions::default();
    let mut categories = Vec::new();
    let mut all = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => options.json = true,
            "--all" => all = true,
            "--runs" => {
                index += 1;
                options.runs = args
                    .get(index)
                    .ok_or_else(|| anyhow!("--runs requires a positive integer"))?
                    .parse()
                    .context("--runs requires a positive integer")?;
                if options.runs == 0 {
                    bail!("--runs requires a positive integer");
                }
            }
            flag => {
                let category = match flag {
                    "--persistence" => BenchmarkCategory::Persistence,
                    "--conversation" => BenchmarkCategory::Conversation,
                    "--serialization" => BenchmarkCategory::Serialization,
                    "--runtime" => BenchmarkCategory::Runtime,
                    "--sessions" => BenchmarkCategory::Sessions,
                    "--tools" => BenchmarkCategory::Tools,
                    "--mutations" => BenchmarkCategory::Mutations,
                    "--mcp" => BenchmarkCategory::Mcp,
                    "--api" => BenchmarkCategory::Api,
                    "--lifecycle" => BenchmarkCategory::Lifecycle,
                    _ => bail!("unknown benchmark option {flag}"),
                };
                categories.push(category);
            }
        }
        index += 1;
    }
    if all {
        options.categories = BenchmarkCategory::all();
    } else if !categories.is_empty() {
        options.categories = BenchmarkCategory::all()
            .into_iter()
            .filter(|category| categories.contains(category))
            .collect();
    }
    Ok(options)
}

fn gateway_url() -> GatewayUrl {
    GatewayUrl::new(config::gateway_url())
}

fn base_url() -> BaseUrl {
    BaseUrl::new(
        env::var("WINDIE_BASE_URL").unwrap_or_else(|_| format!("{}/v1", config::gateway_url())),
    )
}

async fn health(url: &str) -> &'static str {
    match reqwest::Client::new().get(url).send().await {
        Ok(response) if response.status().is_success() => "running",
        _ => "stopped",
    }
}

fn repository_root() -> Result<PathBuf> {
    if let Ok(root) = env::var("WINDIE_DEV_ROOT") {
        let root = PathBuf::from(root);
        if root.join("Cargo.toml").is_file() {
            return Ok(root);
        }
    }
    let current = env::current_dir().context("failed to determine repository root")?;
    if current.join("Cargo.toml").is_file() {
        return Ok(current);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest.to_path_buf())
}

fn npm_command() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}
