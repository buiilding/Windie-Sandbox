//! Installation and integration diagnostics.

use std::env;
use std::path::PathBuf;

use crate::paths;

#[derive(Debug, Clone)]
/// One external integration Windie can launch when its prerequisite exists.
pub struct IntegrationDiagnostic {
    pub name: &'static str,
    pub available: bool,
    pub runtime: &'static str,
    pub install: &'static str,
}

#[derive(Debug, Clone)]
/// Local paths and external runtime availability shown by `windie doctor`.
pub struct DoctorReport {
    pub executable: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub provider_env: PathBuf,
    pub integrations: Vec<IntegrationDiagnostic>,
}

/// Inspects installation state without starting providers or changing files.
pub fn inspect() -> DoctorReport {
    let npx = command_exists("npx");
    DoctorReport {
        executable: env::current_exe().unwrap_or_else(|_| PathBuf::from("windie")),
        data_dir: paths::data_dir(),
        config_dir: paths::config_dir(),
        provider_env: paths::config_dir().join("providers.env"),
        integrations: vec![
            IntegrationDiagnostic {
                name: "Bifrost",
                available: npx || command_exists("docker"),
                runtime: "npx -y @maximhq/bifrost@1.6.3",
                install: "Install Node/npm or Docker, then configure providers at http://localhost:8080",
            },
            IntegrationDiagnostic {
                name: "CUA Driver",
                available: command_exists("cua-driver"),
                runtime: "cua-driver mcp",
                install: "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh)\"",
            },
            IntegrationDiagnostic {
                name: "Desktop Commander",
                available: npx,
                runtime: "npx -y @wonderwhy-er/desktop-commander@0.2.44",
                install: "Install Node/npm; Windie downloads the pinned package on first explicit use",
            },
            IntegrationDiagnostic {
                name: "Blender MCP",
                available: command_exists("uvx"),
                runtime: "uvx --python 3.11 blender-mcp==1.6.0",
                install: "brew install uv, then install and enable the Blender MCP addon in Blender",
            },
        ],
    }
}

fn command_exists(program: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|path| path.join(program).is_file()))
        .unwrap_or(false)
}
