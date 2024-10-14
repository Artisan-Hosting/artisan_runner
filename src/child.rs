use artisan_middleware::{
    common::{log_error, wind_down_state},
    log,
    logger::LogLevel,
    process_manager::ProcessManager,
    state_persistence::AppState,
};
use dusa_collection_utils::{errors::ErrorArrayItem, types::PathType};
use std::{fs, process::Stdio};
use tokio::process::{Child, Command};

use crate::config::AppSpecificConfig;

pub async fn create_child(
    mut state: &mut AppState,
    state_path: &PathType,
    settings: &AppSpecificConfig,
) -> Child {
    log!(LogLevel::Trace, "Creating child process...");

    let mut command = Command::new("npm");

    command
        .args(&["--prefix", &settings.clone().project_path, "run", "start"]) // Updated to run "build" instead of "start"
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NODE_ENV", "production") // Set NODE_ENV=production
        .env("PORT", "3080"); // Set PORT=3000

    match ProcessManager::spawn_complex_process(command, true, true, state, state_path).await {
        Ok(spawned_child) => {
            // read the pid from the state
            let pid: u32 = state.data.replace("PID: ", "").parse::<u32>().unwrap();

            // save the pid somewhere
            let pid_file: PathType = PathType::Content(format!("/tmp/.{}_pg.pid", state.config.app_name));
            
            if let Err(error) = fs::write(pid_file, pid.to_string()) {
                let error_ref = error.get_ref().unwrap_or_else(|| {
                    log!(LogLevel::Trace, "{:?}", error);
                    wind_down_state(state, state_path);
                    std::process::exit(100);
                });
    
                let error_item = ErrorArrayItem::new(
                    dusa_collection_utils::errors::Errors::InputOutput,
                    error_ref.to_string(),
                );
                log_error(&mut state, error_item, &state_path);
                wind_down_state(&mut state, &state_path);
                std::process::exit(100);
            }
            log!(LogLevel::Info, "Child process spawned, pid info saved");
            return spawned_child;
        }
        Err(error) => {
            let error_ref = error.get_ref().unwrap_or_else(|| {
                log!(LogLevel::Trace, "{:?}", error);
                log!(
                    LogLevel::Error,
                    "Child failed to spawn and we couldn't unpack why"
                );
                wind_down_state(state, state_path);
                std::process::exit(100);
            });

            let error_item = ErrorArrayItem::new(
                dusa_collection_utils::errors::Errors::InputOutput,
                error_ref.to_string(),
            );
            log_error(&mut state, error_item, &state_path);
            wind_down_state(&mut state, &state_path);
            std::process::exit(100);
        }
    }
}

pub async fn run_one_shot_process(settings: &AppSpecificConfig) -> Result<(), String> {
    // Set the environment variable NODE_ENV to "production"
    let output = Command::new("npm")
        .arg("--prefix")
        .arg(settings.clone().project_path)
        .arg("run")
        .arg("build")
        .env("NODE_ENV", "production") // Add this line to set NODE_ENV=production
        .output()
        .await
        .map_err(|err| format!("Failed to execute npm run build: {}", err))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    log!(LogLevel::Debug, "Standard Out: {}", stdout);
    log!(LogLevel::Debug, "Standard Err: {}", stderr);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("npm run build failed: {}", stderr));
    }

    Ok(())
}
