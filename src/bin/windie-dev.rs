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
use windie::operation;
use windie::output::TerminalOutput;
use windie::perf::{self, BenchmarkCategory, BenchmarkMode, BenchmarkOptions};

const DEV_GATEWAY_START_TIMEOUT: Duration = Duration::from_secs(180);

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

/// Builds and runs gateway, API, and the HMR Inspector together.
async fn dev_up() -> Result<()> {
    println!("windie-dev: starting gateway");
    let mut gateway = spawn_gateway().await?;
    if let Err(error) = wait_for_gateway(&mut gateway).await {
        stop_child(&mut gateway).await;
        return Err(error);
    }

    let mut api = match spawn_component("api").await {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut gateway).await;
            return Err(error);
        }
    };
    let mut inspector = match spawn_component("inspector").await {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut api).await;
            stop_child(&mut gateway).await;
            return Err(error);
        }
    };
    println!("windie-dev: api and inspector are running; press Ctrl-C to stop");

    let result = supervise_children(&mut gateway, &mut api, &mut inspector).await;
    stop_child(&mut api).await;
    stop_child(&mut inspector).await;
    stop_child(&mut gateway).await;
    result
}

/// Runs one development component in the foreground.
async fn dev_run(component: &str) -> Result<()> {
    match component {
        "gateway" => {
            let mut gateway = spawn_gateway().await?;
            if let Err(error) = wait_for_gateway(&mut gateway).await {
                stop_child(&mut gateway).await;
                return Err(error);
            }
            println!("windie-dev: gateway is running; press Ctrl-C to stop");
            let result = supervise_one(&mut gateway).await;
            stop_child(&mut gateway).await;
            result
        }
        "api" | "inspector" => {
            let mut child = spawn_component(component).await?;
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

/// Builds and starts the current Bifrost source as one foreground process.
async fn spawn_gateway() -> Result<Child> {
    let root = repository_root()?;
    let bifrost_root = root.join("vendor/bifrost");
    let transport_root = bifrost_root.join("transports/bifrost-http");
    if !transport_root.join("main.go").is_file() {
        bail!("Bifrost source is missing at {}", transport_root.display());
    }
    prepare_bifrost_workspace(&bifrost_root).await?;

    let app_dir = windie::local::windie_home_dir()?.join("bifrost/data");
    let port = gateway_url().port();
    let executable = root
        .join("target/windie-dev")
        .join(executable_name("bifrost-http"));
    fs::create_dir_all(
        executable
            .parent()
            .expect("Bifrost binary has a parent directory"),
    )
    .with_context(|| format!("failed to create {}", executable.display()))?;
    let build_status = Command::new("go")
        .args(["build", "-tags", "dev", "-o"])
        .arg(&executable)
        .arg("./transports/bifrost-http")
        .current_dir(&bifrost_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to build Bifrost")?;
    if !build_status.success() {
        bail!("Bifrost build exited with {build_status}");
    }

    let mut command = Command::new(&executable);
    command
        .args(["-host", "127.0.0.1", "-port"])
        .arg(port)
        .arg("-app-dir")
        .arg(app_dir)
        .current_dir(&bifrost_root)
        .env("BIFROST_UI_DEV", "true")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .context("failed to start the Bifrost process")
}

/// Waits for the directly launched Bifrost process to become healthy.
async fn wait_for_gateway(child: &mut Child) -> Result<()> {
    let health_url = format!("{}/health", config::gateway_url());
    for _ in 0..(DEV_GATEWAY_START_TIMEOUT.as_millis() / 200) {
        if health(&health_url).await == "running" {
            return Ok(());
        }
        if let Some(status) = child.try_wait().context("failed to poll Bifrost process")? {
            bail!("Bifrost development process exited with {status}");
        }
        sleep(Duration::from_millis(200)).await;
    }
    bail!(
        "Bifrost did not become healthy within {} seconds",
        DEV_GATEWAY_START_TIMEOUT.as_secs()
    )
}

/// Creates the local Bifrost Go workspace used by the development build.
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

/// Builds and starts one foreground development component.
async fn spawn_component(component: &str) -> Result<Child> {
    let root = repository_root()?;
    let mut command = if component == "inspector" {
        let mut command = Command::new(npm_command());
        command
            .arg("start")
            .arg("--prefix")
            .arg(root.join("dev/windie-inspector"));
        command.env("BROWSER", "none");
        command
    } else if component == "api" {
        let executable = build_windie_binary(&root).await?;
        let mut command = Command::new(executable);
        command.args(["api", "run"]);
        command
    } else {
        bail!("unknown development component {component}");
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

/// Builds the current Windie API executable and returns its debug path.
async fn build_windie_binary(root: &Path) -> Result<PathBuf> {
    let target_directory = cargo_target_directory(root).await?;
    let status = Command::new("cargo")
        .args(["build", "--bin", "windie"])
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("failed to build the Windie API")?;
    if !status.success() {
        bail!("Windie API build exited with {status}");
    }

    let executable = target_directory
        .join("debug")
        .join(executable_name("windie"));
    if !executable.is_file() {
        bail!(
            "Windie API executable was not produced at {}",
            executable.display()
        );
    }
    Ok(executable)
}

/// Finds Cargo's effective target directory for the current workspace.
async fn cargo_target_directory(root: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .await
        .context("failed to inspect the Cargo target directory")?;
    if !output.status.success() {
        bail!(
            "Cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Cargo metadata returned invalid JSON")?;
    let target_directory = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Cargo metadata did not report a target directory"))?;
    Ok(PathBuf::from(target_directory))
}

/// Adds the platform executable suffix used by local development binaries.
fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
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
    let model = benchmark_model().await?;
    let output = TerminalOutput;
    if options.runs == 1 && !options.json {
        let baseline = perf::run(
            mode,
            conversation_id,
            gateway_url(),
            base_url(),
            model.clone(),
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
            model,
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
    let model = benchmark_model().await?;
    let baseline_path = perf::default_baseline_path()?;
    let baseline = perf::read_report(&baseline_path)?;
    let current = perf::run_report(
        BenchmarkMode::Local,
        None,
        gateway_url(),
        base_url(),
        model,
        &options,
    )
    .await?;
    TerminalOutput.performance_comparison(&perf::compare_reports(&baseline, &current));
    Ok(())
}

/// Replaces the checked-in benchmark baseline with a current local run.
async fn update_baseline(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let model = benchmark_model().await?;
    let baseline_path = perf::default_baseline_path()?;
    let report = perf::run_report(
        BenchmarkMode::Local,
        None,
        gateway_url(),
        base_url(),
        model,
        &options,
    )
    .await?;
    perf::write_report(&baseline_path, &report)?;
    TerminalOutput.updated_baseline(&baseline_path);
    Ok(())
}

/// Selects the first model currently exposed by Bifrost for development
/// benchmarks when the caller has not supplied a model override.
async fn benchmark_model() -> Result<ModelName> {
    operation::list_models(gateway_url(), base_url())
        .await?
        .into_iter()
        .next()
        .map(|model| ModelName::new(model.id))
        .ok_or_else(|| anyhow!("no models are available; configure a provider key first"))
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
