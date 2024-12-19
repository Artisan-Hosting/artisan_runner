use artisan_middleware::{
    common::{log_error, update_state, wind_down_state},
    process_manager::{spawn_complex_process, SupervisedChild},
    state_persistence::AppState,
};
use dusa_collection_utils::{errors::ErrorArrayItem, log, types::PathType};
use dusa_collection_utils::log::LogLevel;
use std::{ffi::c_int, fs, process::Stdio};
use tokio::process::Command;

use crate::config::AppSpecificConfig;

pub async fn create_child(
    mut state: &mut AppState,
    state_path: &PathType,
    settings: &AppSpecificConfig,
) -> SupervisedChild {
    log!(LogLevel::Trace, "Creating child process...");

    let mut command = Command::new("npm");

    command
        .args(&["--prefix", &settings.clone().project_path, "run", "start"]) // Updated to run "build" instead of "start"
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NODE_ENV", "production") // Set NODE_ENV=production
        .env("PORT", "3080"); // Set PORT=3000


    match spawn_complex_process(command, false, true).await { //TODO change this back
        Ok(spawned_child) => {
            // initialize monitor loop.
            spawned_child.monitor_usage().await;
            // read the pid from the state
            let pid: u32 = match spawned_child.get_pid().await {
                Ok(xid) => xid,
                Err(_) => {
                    let error_item = ErrorArrayItem::new(
                        dusa_collection_utils::errors::Errors::InputOutput,
                        "No pid for supervised child".to_owned(),
                    );
                    log_error(state, error_item, &state_path).await;
                    wind_down_state(state, &state_path).await;
                    std::process::exit(100);
                }
            };

            // save the pid somewhere
            let pid_file: PathType =
                PathType::Content(format!("/tmp/.{}_pg.pid", state.config.app_name));

            if let Err(error) = fs::write(pid_file, pid.to_string()) {
                let error_ref = error.get_ref().unwrap_or_else(|| {
                    log!(LogLevel::Trace, "{:?}", error);
                    std::process::exit(100);
                });

                let error_item = ErrorArrayItem::new(
                    dusa_collection_utils::errors::Errors::InputOutput,
                    error_ref.to_string(),
                );
                log_error(&mut state, error_item, &state_path).await;
                wind_down_state(&mut state, &state_path).await;
                std::process::exit(100);
            }
            log!(LogLevel::Info, "Child process spawned, pid info saved");

            if let Ok(metrics) = spawned_child.get_metrics().await {
                update_state(&mut state, &state_path, Some(metrics)).await;
            }
            return spawned_child;
        }
        Err(error) => {
            log_error(&mut state, error, &state_path).await;
            wind_down_state(&mut state, &state_path).await;
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
        .env("NODE_ENV", "production") 
        .output()
        .await
        .map_err(|err| format!("Failed to execute npm run build: {}", err))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    log!(LogLevel::Debug, "Standard Out: {}", stdout);
    log!(LogLevel::Debug, "Standard Err: {}", stderr);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Oneshot process failed: {}", stderr));
    }

    Ok(())
}

pub fn _get_pid(state: &mut AppState) -> Result<c_int, ErrorArrayItem>{
    let pid_file: PathType =
    PathType::Content(format!("/tmp/.{}_pg.pid", state.config.app_name));


    let data = match fs::read_to_string(pid_file) {
        Ok(data) => data.trim_end().replace(" ", ""),
        Err(err) => return Err(ErrorArrayItem::from(err)),
    };

    let pid_number = match data.parse::<c_int>() {
        Ok(int) => int,
        Err(err) => return Err(ErrorArrayItem::from(err)),
    };

    Ok(pid_number)
}



// .parse::<c_int>() 