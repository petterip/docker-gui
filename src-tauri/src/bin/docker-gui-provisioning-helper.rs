use bollard::volume::CreateVolumeOptions;
use bollard::volume::RemoveVolumeOptions;
use bollard::Docker;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Stdio;
#[cfg(target_os = "windows")]
use tokio::io;
#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
#[cfg(target_os = "windows")]
use tokio::process::Command as TokioCommand;
#[cfg(target_os = "windows")]
use tokio::time::{sleep, Duration};
use std::sync::OnceLock;

#[derive(Debug, Serialize)]
struct HelperActionResponse {
    status: &'static str,
    details: Option<Value>,
    failure_class: Option<&'static str>,
    message: Option<String>,
    retriable: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Args {
    action: String,
    target_json: Value,
    app_data_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct RelayArgs {
    distro: String,
    relay_pipe: String,
    app_data_dir: Option<String>,
}

enum HelperCliCommand {
    RunAction(Args),
    RunRelay(RelayArgs),
}

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

#[tokio::main]
async fn main() {
    let command = match parse_args() {
        Ok(cmd) => cmd,
        Err(e) => {
            print_response(HelperActionResponse {
                status: "failed",
                details: None,
                failure_class: Some("helper_failed"),
                message: Some(e),
                retriable: Some(false),
            });
            std::process::exit(2);
        }
    };

    match command {
        HelperCliCommand::RunAction(args) => {
            init_logger(args.app_data_dir.as_deref());
            log_helper(&format!(
                "run-action start action={} target_provider={}",
                args.action,
                args.target_json
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
            let action_name = args.action.clone();
            let result = dispatch(args).await;
            match result {
                Ok(details) => {
                    log_helper(&format!("run-action success action={action_name}"));
                    print_response(HelperActionResponse {
                        status: "succeeded",
                        details: Some(details),
                        failure_class: None,
                        message: None,
                        retriable: None,
                    })
                }
                Err((class, message, retriable)) => {
                    log_helper(&format!(
                        "run-action failed class={class} message={message} retriable={retriable}"
                    ));
                    print_response(HelperActionResponse {
                        status: "failed",
                        details: None,
                        failure_class: Some(class),
                        message: Some(message),
                        retriable: Some(retriable),
                    });
                    std::process::exit(1);
                }
            }
        }
        HelperCliCommand::RunRelay(args) => {
            init_logger(args.app_data_dir.as_deref());
            log_helper(&format!(
                "run-relay start distro={} pipe={}",
                args.distro, args.relay_pipe
            ));
            if let Err(e) = run_relay_daemon(args).await {
                log_helper(&format!("run-relay failed: {e}"));
                eprintln!("{e}");
                std::process::exit(1);
            }
            log_helper("run-relay finished");
        }
    }
}

fn print_response(response: HelperActionResponse) {
    if let Ok(out) = serde_json::to_string(&response) {
        println!("{out}");
    }
}

fn parse_args() -> Result<HelperCliCommand, String> {
    let mut action = None::<String>;
    let mut target_json = None::<Value>;
    let mut app_data_dir = None::<String>;
    let mut distro = None::<String>;
    let mut relay_pipe = None::<String>;

    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or_else(|| "missing subcommand".to_string())?;

    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--action" => {
                action = it.next();
            }
            "--target-json" => {
                let raw = it
                    .next()
                    .ok_or_else(|| "missing --target-json value".to_string())?;
                target_json =
                    Some(serde_json::from_str(&raw).map_err(|e| format!("invalid target json: {e}"))?);
            }
            "--target-provider" => {
                let _ = it.next();
            }
            "--app-data-dir" => {
                app_data_dir = it.next();
            }
            "--distro" => {
                distro = it.next();
            }
            "--pipe" => {
                relay_pipe = it.next();
            }
            _ => {}
        }
    }

    match cmd.as_str() {
        "run-action" => Ok(HelperCliCommand::RunAction(Args {
            action: action.ok_or_else(|| "missing --action".to_string())?,
            target_json: target_json.ok_or_else(|| "missing --target-json".to_string())?,
            app_data_dir,
        })),
        "run-relay" => Ok(HelperCliCommand::RunRelay(RelayArgs {
            distro: distro
                .ok_or_else(|| "missing --distro".to_string())?
                .trim()
                .to_string(),
            relay_pipe: relay_pipe
                .ok_or_else(|| "missing --pipe".to_string())?
                .trim()
                .to_string(),
            app_data_dir,
        })),
        _ => Err("unsupported subcommand".to_string()),
    }
}

async fn dispatch(args: Args) -> Result<Value, (&'static str, String, bool)> {
    log_helper(&format!(
        "dispatch action={} target={}",
        args.action, args.target_json
    ));
    match args.action.as_str() {
        "host_engine_detect" => {
            let endpoint = target_endpoint(&args.target_json)?;
            if can_ping(&endpoint).await {
                log_helper(&format!("host_engine_detect succeeded endpoint={endpoint}"));
                Ok(serde_json::json!({ "endpoint": endpoint }))
            } else {
                log_helper("host_engine_detect found no compatible host");
                Err((
                    "host_not_installed",
                    "No compatible Host Engine was detected.".to_string(),
                    false,
                ))
            }
        }
        "host_compatibility_validate" => {
            let endpoint = target_endpoint(&args.target_json)?;
            validate_host_compatibility(&endpoint).await?;
            log_helper(&format!(
                "host_compatibility_validate succeeded endpoint={endpoint}"
            ));
            Ok(serde_json::json!({ "endpoint": endpoint }))
        }
        "wsl_prereq_enable" => {
            ensure_windows_wsl_prerequisites()?;
            Ok(serde_json::json!({}))
        }
        "wsl_distro_install" => {
            ensure_wsl_distro_ready(&args.target_json)?;
            Ok(serde_json::json!({}))
        }
        "wsl_engine_install" => {
            let distro = target_distro(&args.target_json)?;
            run_wsl_engine_install_script(&distro)?;
            Ok(serde_json::json!({ "distro": distro }))
        }
        "wsl_relay_register" => {
            let distro = target_distro(&args.target_json)?;
            let relay_pipe = target_relay_pipe(&args.target_json)?;
            verify_wsl_engine_socket_ready(&distro)?;
            let app_data_dir = args
                .app_data_dir
                .ok_or_else(|| ("helper_failed", "missing app data dir".to_string(), false))?;
            register_wsl_relay(Path::new(&app_data_dir), &distro, &relay_pipe)?;
            ensure_wsl_relay_registration_and_health(Path::new(&app_data_dir), &distro, &relay_pipe)
                .await?;
            Ok(serde_json::json!({ "distro": distro, "relay_pipe": relay_pipe }))
        }
        "wsl_managed_distro_remove" => {
            unregister_managed_wsl_distro()?;
            Ok(serde_json::json!({}))
        }
        _ => Err((
            "helper_failed",
            format!("unsupported action: {}", args.action),
            false,
        )),
    }
}

fn target_endpoint(target_json: &Value) -> Result<String, (&'static str, String, bool)> {
    let endpoint = target_json
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if endpoint.is_empty() {
        Err(("connectivity_failed", "Target endpoint is missing.".to_string(), false))
    } else {
        Ok(endpoint)
    }
}

fn target_distro(target_json: &Value) -> Result<String, (&'static str, String, bool)> {
    let distro = target_json
        .get("distro")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if distro.is_empty() {
        Err(("prereq_missing", "Managed WSL distro is not configured.".to_string(), false))
    } else {
        Ok(distro)
    }
}

fn target_relay_pipe(target_json: &Value) -> Result<String, (&'static str, String, bool)> {
    let relay_pipe = target_json
        .get("relay_pipe")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if relay_pipe.is_empty() {
        Err(("relay_failed", "Relay endpoint is missing.".to_string(), false))
    } else {
        Ok(relay_pipe)
    }
}

async fn can_ping(endpoint: &str) -> bool {
    let docker = match connect_docker(endpoint) {
        Some(docker) => docker,
        None => return false,
    };
    docker.ping().await.is_ok()
}

#[cfg(target_os = "windows")]
fn connect_docker(endpoint: &str) -> Option<Docker> {
    if endpoint.starts_with("npipe:")
        || endpoint.starts_with("npipe://")
        || endpoint.starts_with("//./pipe/")
        || endpoint.starts_with("\\\\.\\pipe\\")
    {
        return Docker::connect_with_named_pipe(
            endpoint,
            120,
            bollard::API_DEFAULT_VERSION,
        )
        .ok();
    }

    if endpoint.starts_with("tcp://") || endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Docker::connect_with_http(endpoint, 120, bollard::API_DEFAULT_VERSION).ok();
    }

    None
}

#[cfg(not(target_os = "windows"))]
fn connect_docker(endpoint: &str) -> Option<Docker> {
    if endpoint.starts_with("unix://") || endpoint.starts_with('/') {
        let unix_path = endpoint.strip_prefix("unix://").unwrap_or(endpoint);
        return Docker::connect_with_unix(unix_path, 120, bollard::API_DEFAULT_VERSION).ok();
    }

    if endpoint.starts_with("tcp://") || endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Docker::connect_with_http(endpoint, 120, bollard::API_DEFAULT_VERSION).ok();
    }

    None
}

async fn validate_host_compatibility(endpoint: &str) -> Result<(), (&'static str, String, bool)> {
    let docker = connect_docker(endpoint).ok_or((
        "host_compat_failed",
        "Could not connect to Host Engine endpoint.".to_string(),
        true,
    ))?;

    docker
        .ping()
        .await
        .map_err(|_| ("host_compat_failed", "Host Engine ping failed.".to_string(), true))?;
    docker
        .version()
        .await
        .map_err(|_| ("host_compat_failed", "Host Engine version query failed.".to_string(), true))?;
    docker
        .info()
        .await
        .map_err(|_| ("host_compat_failed", "Host Engine info query failed.".to_string(), true))?;

    let probe_volume = format!("docker_gui_probe_{}", uuid::Uuid::new_v4());
    docker
        .create_volume(CreateVolumeOptions {
            name: probe_volume.clone(),
            driver: "local".to_string(),
            ..Default::default()
        })
        .await
        .map_err(|_| {
            (
                "host_compat_failed",
                "Host Engine volume create check failed.".to_string(),
                true,
            )
        })?;
    docker
        .remove_volume(&probe_volume, Some(RemoveVolumeOptions { force: true }))
        .await
        .map_err(|_| {
            (
                "host_compat_failed",
                "Host Engine volume remove check failed.".to_string(),
                true,
            )
        })?;

    if !host_compose_available() {
        return Err((
            "host_compat_failed",
            "Host Engine Compose compatibility check failed.".to_string(),
            false,
        ));
    }

    Ok(())
}

fn host_compose_available() -> bool {
    let v2 = Command::new("docker").args(["compose", "version"]).output();
    if v2.map(|o| o.status.success()).unwrap_or(false) {
        return true;
    }
    let v1 = Command::new("docker-compose").arg("version").output();
    v1.map(|o| o.status.success()).unwrap_or(false)
}

fn init_logger(app_data_dir: Option<&str>) {
    let _ = LOG_PATH.set(
        app_data_dir
            .and_then(|dir| {
                let logs_dir = Path::new(dir).join("logs");
                std::fs::create_dir_all(&logs_dir).ok()?;
                Some(logs_dir.join("provisioning-helper.log"))
            }),
    );
}

fn log_helper(message: &str) {
    if let Some(Some(path)) = LOG_PATH.get() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[{}] {}", Utc::now().to_rfc3339(), message);
        }
    }
}

fn log_command_failure(cmd: &str, args: &[&str], output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    log_helper(&format!(
        "command failed: {} {} exit_code={} stdout={} stderr={}",
        cmd,
        args.join(" "),
        output.status.code().unwrap_or(-1),
        stdout.trim(),
        stderr.trim()
    ));
}

#[cfg(target_os = "windows")]
fn ensure_windows_wsl_prerequisites() -> Result<(), (&'static str, String, bool)> {
    if wsl_status_ok() {
        return Ok(());
    }
    let _ = run_windows_command("wsl", &["--install", "--no-distribution"]);
    let _ = run_windows_command(
        "dism.exe",
        &[
            "/online",
            "/enable-feature",
            "/featurename:Microsoft-Windows-Subsystem-Linux",
            "/all",
            "/norestart",
        ],
    );
    let _ = run_windows_command(
        "dism.exe",
        &[
            "/online",
            "/enable-feature",
            "/featurename:VirtualMachinePlatform",
            "/all",
            "/norestart",
        ],
    );

    if wsl_status_ok() {
        Ok(())
    } else {
        Err((
            "prereq_missing",
            "WSL prerequisites are still missing.".to_string(),
            false,
        ))
    }
}

#[cfg(not(target_os = "windows"))]
fn ensure_windows_wsl_prerequisites() -> Result<(), (&'static str, String, bool)> {
    Err(("prereq_missing", "WSL provisioning is only supported on Windows.".to_string(), false))
}

#[cfg(target_os = "windows")]
fn wsl_status_ok() -> bool {
    Command::new("wsl")
        .arg("--status")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn run_windows_command(cmd: &str, args: &[&str]) -> Result<(), (&'static str, String, bool)> {
    log_helper(&format!("run_windows_command: {} {}", cmd, args.join(" ")));
    let output = Command::new(cmd).args(args).output().map_err(|e| {
        (
            "prereq_missing",
            format!("Could not execute prerequisite command {cmd}: {e}"),
            false,
        )
    })?;
    if output.status.success() {
        log_helper(&format!(
            "run_windows_command succeeded: {} exit_code={}",
            cmd,
            output.status.code().unwrap_or(-1)
        ));
        return Ok(());
    }
    log_command_failure(cmd, args, &output);
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    if combined.contains("restart") || combined.contains("reboot") {
        return Err(("reboot_required", "Windows restart is required.".to_string(), false));
    }
    if combined.contains("access is denied") || combined.contains("administrator") {
        return Err(("permission_denied", "Administrator permissions are required.".to_string(), false));
    }
    Err(("prereq_missing", "WSL prerequisite command failed.".to_string(), true))
}

#[cfg(target_os = "windows")]
fn ensure_wsl_distro_ready(target_json: &Value) -> Result<(), (&'static str, String, bool)> {
    let preferred = target_distro(target_json)?;
    let installed = list_wsl_distros()?;
    if installed.iter().any(|d| d.eq_ignore_ascii_case(&preferred))
        || installed.iter().any(|d| is_supported_ubuntu_distro(d))
    {
        return Ok(());
    }
    install_supported_wsl_distro()?;
    let installed_after = list_wsl_distros()?;
    if installed_after.iter().any(|d| is_supported_ubuntu_distro(d)) {
        Ok(())
    } else {
        Err(("distro_install_failed", "WSL distro installation did not complete.".to_string(), false))
    }
}

#[cfg(not(target_os = "windows"))]
fn ensure_wsl_distro_ready(_target_json: &Value) -> Result<(), (&'static str, String, bool)> {
    Err(("prereq_missing", "WSL provisioning is only supported on Windows.".to_string(), false))
}

#[cfg(target_os = "windows")]
fn list_wsl_distros() -> Result<Vec<String>, (&'static str, String, bool)> {
    let output = Command::new("wsl").args(["-l", "-q"]).output().map_err(|e| {
        (
            "prereq_missing",
            format!("Unable to query WSL distros: {e}"),
            false,
        )
    })?;
    if !output.status.success() {
        return Err(("prereq_missing", "Unable to query WSL distros.".to_string(), false));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

#[cfg(target_os = "windows")]
fn install_supported_wsl_distro() -> Result<(), (&'static str, String, bool)> {
    log_helper("install_supported_wsl_distro starting");
    let output = Command::new("wsl")
        .args(["--install", "-d", "Ubuntu"])
        .output()
        .map_err(|e| ("distro_install_failed", format!("Could not install WSL distro: {e}"), false))?;
    if output.status.success() {
        log_helper("install_supported_wsl_distro succeeded");
        return Ok(());
    }
    log_command_failure("wsl", &["--install", "-d", "Ubuntu"], &output);
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    if combined.contains("restart") || combined.contains("reboot") {
        return Err(("reboot_required", "Windows restart is required.".to_string(), false));
    }
    if combined.contains("access is denied") || combined.contains("administrator") {
        return Err(("permission_denied", "Administrator permissions are required.".to_string(), false));
    }
    Err(("distro_install_failed", "Could not install a supported WSL distro.".to_string(), false))
}

#[cfg(target_os = "windows")]
fn run_wsl_engine_install_script(distro: &str) -> Result<(), (&'static str, String, bool)> {
    log_helper(&format!(
        "run_wsl_engine_install_script start distro={distro}"
    ));
    let script = r#"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y ca-certificates curl gnupg lsb-release
apt-get install -y docker.io docker-compose-plugin || apt-get install -y docker.io docker-compose
groupadd -f docker || true
DEFAULT_USER="$(getent passwd 1000 | cut -d: -f1 || true)"
if [ -n "${DEFAULT_USER}" ] && id "${DEFAULT_USER}" >/dev/null 2>&1; then
  usermod -aG docker "${DEFAULT_USER}" || true
fi
service docker start || true
if command -v systemctl >/dev/null 2>&1; then
  systemctl enable docker || true
  systemctl start docker || true
fi
docker version >/dev/null 2>&1
docker info >/dev/null 2>&1
docker compose version >/dev/null 2>&1 || docker-compose version >/dev/null 2>&1
"#;

    let output = Command::new("wsl")
        .args(["-d", distro, "-u", "root", "--", "bash", "-lc", script])
        .output()
        .map_err(|e| {
            (
                "engine_install_failed",
                format!("Failed to run engine installation in WSL distro {distro}: {e}"),
                true,
            )
        })?;
    if output.status.success() {
        log_helper(&format!(
            "run_wsl_engine_install_script succeeded distro={distro}"
        ));
        return Ok(());
    }
    log_command_failure("wsl", &["-d", distro, "-u", "root", "--", "bash", "-lc", script], &output);
    Err((
        "engine_install_failed",
        "Engine package installation in WSL did not complete.".to_string(),
        true,
    ))
}

#[cfg(not(target_os = "windows"))]
fn run_wsl_engine_install_script(_distro: &str) -> Result<(), (&'static str, String, bool)> {
    Err(("prereq_missing", "WSL provisioning is only supported on Windows.".to_string(), false))
}

#[cfg(target_os = "windows")]
fn verify_wsl_engine_socket_ready(distro: &str) -> Result<(), (&'static str, String, bool)> {
    ensure_wsl_engine_runtime_running(distro)?;

    let output = Command::new("wsl")
        .args([
            "-d",
            distro,
            "--",
            "bash",
            "-lc",
            "test -S /var/run/docker.sock && docker info >/dev/null 2>&1",
        ])
        .output()
        .map_err(|e| ("relay_failed", format!("Could not verify WSL socket: {e}"), true))?;
    if output.status.success() {
        log_helper(&format!(
            "verify_wsl_engine_socket_ready succeeded distro={distro}"
        ));
        Ok(())
    } else {
        log_command_failure(
            "wsl",
            &[
                "-d",
                distro,
                "--",
                "bash",
                "-lc",
                "test -S /var/run/docker.sock && docker info >/dev/null 2>&1",
            ],
            &output,
        );
        Err(("relay_failed", "WSL engine socket is not ready for relay registration.".to_string(), true))
    }
}

#[cfg(not(target_os = "windows"))]
fn verify_wsl_engine_socket_ready(_distro: &str) -> Result<(), (&'static str, String, bool)> {
    Err(("prereq_missing", "WSL provisioning is only supported on Windows.".to_string(), false))
}

#[cfg(target_os = "windows")]
fn ensure_wsl_engine_runtime_running(distro: &str) -> Result<(), (&'static str, String, bool)> {
    let output = Command::new("wsl")
        .args([
            "-d",
            distro,
            "-u",
            "root",
            "--",
            "bash",
            "-lc",
            r#"set -e
service docker start >/dev/null 2>&1 || true
if command -v systemctl >/dev/null 2>&1; then
  systemctl start docker >/dev/null 2>&1 || true
fi
docker info >/dev/null 2>&1"#,
        ])
        .output()
        .map_err(|e| {
            (
                "engine_start_failed",
                format!("Could not start Docker engine service in WSL distro {distro}: {e}"),
                true,
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();

    if combined.contains("permission denied")
        || combined.contains("access is denied")
        || combined.contains("administrator")
    {
        return Err((
            "permission_denied",
            "Administrator permissions are required to start Docker service in WSL.".to_string(),
            false,
        ));
    }

    Err((
        "engine_start_failed",
        "Docker engine service in WSL is not running.".to_string(),
        true,
    ))
}

fn register_wsl_relay(
    app_data_dir: &Path,
    distro: &str,
    relay_pipe: &str,
) -> Result<(), (&'static str, String, bool)> {
    log_helper(&format!(
        "register_wsl_relay start distro={} relay_pipe={}",
        distro, relay_pipe
    ));
    std::fs::create_dir_all(app_data_dir).map_err(|e| {
        (
            "relay_failed",
            format!("Could not create relay registration directory: {e}"),
            false,
        )
    })?;
    let relay_path = app_data_dir.join("wsl_relay_registration.json");
    let payload = serde_json::json!({
        "provider": "wsl_engine",
        "distro": distro,
        "relay_pipe": relay_pipe,
        "registered_at": chrono::Utc::now().to_rfc3339(),
    });
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| ("relay_failed", format!("Could not serialize relay payload: {e}"), false))?;
    std::fs::write(&relay_path, content).map_err(|e| {
        (
            "relay_failed",
            format!("Could not persist relay registration metadata: {e}"),
            false,
        )
    })?;
    log_helper("register_wsl_relay completed");
    Ok(())
}

#[cfg(target_os = "windows")]
fn update_wsl_relay_state(
    app_data_dir: &Path,
    distro: &str,
    relay_pipe: &str,
    status: &str,
    last_error: Option<String>,
) -> Result<(), (&'static str, String, bool)> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| {
        (
            "relay_failed",
            format!("Could not create relay state directory: {e}"),
            false,
        )
    })?;
    let relay_state_path = app_data_dir.join("wsl_relay_state.json");
    let payload = serde_json::json!({
        "provider": "wsl_engine",
        "distro": distro,
        "relay_pipe": relay_pipe,
        "status": status,
        "last_checked_at": chrono::Utc::now().to_rfc3339(),
        "last_error": last_error,
    });
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| ("relay_failed", format!("Could not serialize relay state payload: {e}"), false))?;
    std::fs::write(relay_state_path, content).map_err(|e| {
        (
            "relay_failed",
            format!("Could not persist relay state metadata: {e}"),
            false,
        )
    })
}

#[cfg(target_os = "windows")]
async fn ensure_wsl_relay_registration_and_health(
    app_data_dir: &Path,
    distro: &str,
    relay_pipe: &str,
) -> Result<(), (&'static str, String, bool)> {
    if can_ping(relay_pipe).await {
        update_wsl_relay_state(app_data_dir, distro, relay_pipe, "running", None)?;
        return Ok(());
    }

    let started_relay = start_managed_wsl_relay_process(app_data_dir, distro, relay_pipe)?;
    if started_relay {
        if wait_for_relay_pipe(relay_pipe, 12, 500).await {
            update_wsl_relay_state(app_data_dir, distro, relay_pipe, "running", None)?;
            return Ok(());
        }
        update_wsl_relay_state(
            app_data_dir,
            distro,
            relay_pipe,
            "degraded",
            Some("Managed relay start command did not produce a reachable endpoint.".to_string()),
        )?;
    }

    if let Some(fallback) = windows_fallback_endpoint() {
        if can_ping(&fallback).await {
            update_wsl_relay_state(
                app_data_dir,
                distro,
                relay_pipe,
                "degraded",
                Some(format!(
                    "Managed relay unavailable, fallback endpoint in use: {fallback}"
                )),
            )?;
            return Ok(());
        }
    }

    update_wsl_relay_state(
        app_data_dir,
        distro,
        relay_pipe,
        "degraded",
        Some("Relay endpoint did not respond after registration.".to_string()),
    )?;
    Err((
        "relay_failed",
        "Relay endpoint is registered but not reachable.".to_string(),
        true,
    ))
}

#[cfg(not(target_os = "windows"))]
async fn ensure_wsl_relay_registration_and_health(
    _app_data_dir: &Path,
    _distro: &str,
    _relay_pipe: &str,
) -> Result<(), (&'static str, String, bool)> {
    Err(("prereq_missing", "WSL provisioning is only supported on Windows.".to_string(), false))
}

#[cfg(target_os = "windows")]
fn start_managed_wsl_relay_process(
    app_data_dir: &Path,
    distro: &str,
    relay_pipe: &str,
) -> Result<bool, (&'static str, String, bool)> {
    let app_data = app_data_dir.to_string_lossy().to_string();
    if let Some(template) = relay_start_command_template() {
        let command = render_relay_start_command(&template, distro, relay_pipe, &app_data);
        let result = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &command,
            ])
            .spawn();
        match result {
            Ok(_) => return Ok(true),
            Err(e) => {
                return Err((
                    "relay_failed",
                    format!("Could not start managed relay process with configured command: {e}"),
                    true,
                ));
            }
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        let result = Command::new(current_exe)
            .args([
                "run-relay",
                "--distro",
                distro,
                "--pipe",
                relay_pipe,
                "--app-data-dir",
                &app_data,
            ])
            .spawn();
        match result {
            Ok(_) => return Ok(true),
            Err(e) => {
                return Err((
                    "relay_failed",
                    format!("Could not start built-in helper relay process: {e}"),
                    true,
                ));
            }
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            for candidate in [
                "docker-gui-wsl-relay.exe",
                "docker-gui-wsl-relay",
                "docker-gui-relay.exe",
                "docker-gui-relay",
            ] {
                let relay_exe = parent.join(candidate);
                if !relay_exe.exists() {
                    continue;
                }
                let result = Command::new(relay_exe)
                    .args([
                        "--distro",
                        distro,
                        "--pipe",
                        relay_pipe,
                        "--app-data-dir",
                        &app_data,
                    ])
                    .spawn();
                match result {
                    Ok(_) => return Ok(true),
                    Err(e) => {
                        return Err((
                            "relay_failed",
                            format!("Could not launch managed relay executable: {e}"),
                            true,
                        ));
                    }
                }
            }
        }
    }
    Ok(false)
}

#[cfg(target_os = "windows")]
fn relay_start_command_template() -> Option<String> {
    std::env::var("DOCKER_GUI_WSL_RELAY_START_CMD")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(target_os = "windows")]
fn render_relay_start_command(
    template: &str,
    distro: &str,
    relay_pipe: &str,
    app_data_dir: &str,
) -> String {
    template
        .replace("{distro}", distro)
        .replace("{relay_pipe}", relay_pipe)
        .replace("{app_data_dir}", app_data_dir)
}

#[cfg(target_os = "windows")]
async fn wait_for_relay_pipe(relay_pipe: &str, attempts: usize, delay_ms: u64) -> bool {
    for _ in 0..attempts {
        if can_ping(relay_pipe).await {
            return true;
        }
        sleep(Duration::from_millis(delay_ms)).await;
    }
    false
}

#[cfg(target_os = "windows")]
async fn run_relay_daemon(args: RelayArgs) -> Result<(), String> {
    let pipe_path = normalize_named_pipe_path(&args.relay_pipe)?;
    let distro = args.distro.trim().to_string();
    if distro.is_empty() {
        return Err("run-relay requires non-empty --distro".to_string());
    }

    if let Some(app_data_dir) = args.app_data_dir.as_deref() {
        let logs_dir = Path::new(app_data_dir).join("logs");
        let _ = std::fs::create_dir_all(logs_dir);
    }

    let mut listener = match create_named_pipe_listener(&pipe_path, true) {
        Ok(listener) => listener,
        Err(e) if e == "relay_already_running" => return Ok(()),
        Err(e) => return Err(e),
    };
    loop {
        listener.connect().await.map_err(|e| {
            format!("relay listener failed to accept pipe client on {pipe_path}: {e}")
        })?;
        let next = create_named_pipe_listener(&pipe_path, false)?;
        let distro_for_conn = distro.clone();
        tokio::spawn(async move {
            let _ = proxy_pipe_connection(listener, &distro_for_conn).await;
        });
        listener = next;
    }
}

#[cfg(not(target_os = "windows"))]
async fn run_relay_daemon(_args: RelayArgs) -> Result<(), String> {
    Err("run-relay is only supported on Windows".to_string())
}

#[cfg(target_os = "windows")]
fn create_named_pipe_listener(path: &str, first: bool) -> Result<NamedPipeServer, String> {
    let mut options = ServerOptions::new();
    options.access_inbound(true).access_outbound(true);
    if first {
        options.first_pipe_instance(true);
    }

    options.create(path).map_err(|e| {
        if first && e.kind() == std::io::ErrorKind::AlreadyExists {
            // Relay already running for this pipe; treat as success for this process.
            "relay_already_running".to_string()
        } else {
            format!("failed creating named pipe {path}: {e}")
        }
    })
}

#[cfg(target_os = "windows")]
async fn proxy_pipe_connection(pipe: NamedPipeServer, distro: &str) -> Result<(), String> {
    let mut child = TokioCommand::new("wsl")
        .args(["-d", distro, "--", "docker", "system", "dial-stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed spawning WSL relay backend process: {e}"))?;

    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open backend stdin".to_string())?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to open backend stdout".to_string())?;

    let (mut pipe_read, mut pipe_write) = io::split(pipe);
    let to_backend = tokio::spawn(async move { io::copy(&mut pipe_read, &mut child_stdin).await });
    let from_backend =
        tokio::spawn(async move { io::copy(&mut child_stdout, &mut pipe_write).await });

    let _ = tokio::join!(to_backend, from_backend);
    let _ = child.kill().await;
    Ok(())
}

#[cfg(target_os = "windows")]
fn normalize_named_pipe_path(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("empty relay pipe value".to_string());
    }

    if value.starts_with(r"\\.\pipe\") {
        return Ok(value.to_string());
    }

    if let Some(idx) = value.find("/pipe/") {
        let name = value[(idx + "/pipe/".len())..].trim().trim_matches('/');
        if name.is_empty() {
            return Err(format!("invalid relay pipe value: {value}"));
        }
        return Ok(format!(r"\\.\pipe\{}", name.replace('/', "\\")));
    }

    if value.starts_with("//./pipe/") {
        let name = value.trim_start_matches("//./pipe/").trim_matches('/');
        if name.is_empty() {
            return Err(format!("invalid relay pipe value: {value}"));
        }
        return Ok(format!(r"\\.\pipe\{}", name.replace('/', "\\")));
    }

    Err(format!("unsupported relay pipe format: {value}"))
}

#[cfg(target_os = "windows")]
fn windows_fallback_endpoint() -> Option<String> {
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        if !host.trim().is_empty() {
            return Some(host);
        }
    }

    let show = Command::new("docker")
        .args(["context", "show"])
        .output()
        .ok()?;
    if show.status.success() {
        let context = String::from_utf8(show.stdout).ok()?.trim().to_string();
        if !context.is_empty() {
            let inspect = Command::new("docker")
                .args([
                    "context",
                    "inspect",
                    "--format",
                    "{{.Endpoints.docker.Host}}",
                    &context,
                ])
                .output()
                .ok()?;
            if inspect.status.success() {
                let host = String::from_utf8(inspect.stdout).ok()?.trim().to_string();
                if !host.is_empty() {
                    return Some(host);
                }
            }
        }
    }

    Some("npipe:////./pipe/docker_engine".to_string())
}

#[cfg(target_os = "windows")]
fn unregister_managed_wsl_distro() -> Result<(), (&'static str, String, bool)> {
    const DEFAULT_WSL_DISTRO: &str = "docker-gui-engine";
    let output = Command::new("wsl")
        .args(["--unregister", DEFAULT_WSL_DISTRO])
        .output()
        .map_err(|e| {
            (
                "distro_remove_failed",
                format!("Could not remove managed WSL distro: {e}"),
                true,
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    if combined.contains("not found") || combined.contains("there is no distribution") {
        return Ok(());
    }
    if combined.contains("access is denied") || combined.contains("administrator") {
        return Err(("permission_denied", "Administrator permissions are required.".to_string(), false));
    }
    Err((
        "distro_remove_failed",
        "Managed engine removal did not complete.".to_string(),
        true,
    ))
}

#[cfg(not(target_os = "windows"))]
fn unregister_managed_wsl_distro() -> Result<(), (&'static str, String, bool)> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn is_supported_ubuntu_distro(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized == "ubuntu" || normalized.starts_with("ubuntu-")
}
