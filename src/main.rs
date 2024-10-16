use artisan_middleware::{
    common::{log_error, update_state, wind_down_state},
    config::AppConfig,
    log,
    logger::{set_log_level, LogLevel},
    process_manager::SupervisedChild,
    state_persistence::{AppState, StatePersistence},
    timestamp::current_timestamp,
};
use child::{create_child, run_one_shot_process};
use config::{get_config, specific_config};
use dusa_collection_utils::{
    errors::{ErrorArrayItem, Errors},
    types::PathType,
};
use monitor::monitor_directory;
use signals::{sighup_watch, sigusr_watch};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

mod child;
mod config;
mod monitor;
mod signals;

#[tokio::main]
async fn main() {
    // Initialization
    log!(LogLevel::Trace, "Initializing application...");
    let mut config: AppConfig = get_config();
    let state_path: PathType = StatePersistence::get_state_path(&config);

    log!(LogLevel::Trace, "Loading specific configuration...");
    let settings = match specific_config() {
        Ok(loaded_data) => {
            log!(
                LogLevel::Trace,
                "Loaded specific configuration successfully"
            );
            loaded_data
        }
        Err(e) => {
            log!(LogLevel::Error, "Error loading settings: {}", e);
            std::process::exit(0)
        }
    };

    // Setting up the state of the application
    log!(LogLevel::Trace, "Setting up the application state...");
    let mut state: AppState = match StatePersistence::load_state(&state_path) {
        Ok(mut loaded_data) => {
            log!(LogLevel::Info, "Loaded previous state data");
            log!(LogLevel::Trace, "Previous state data: {:#?}", loaded_data);
            loaded_data.is_active = false;
            loaded_data.data = String::from("Initializing");
            loaded_data.config.debug_mode = config.debug_mode;
            loaded_data.last_updated = current_timestamp();
            loaded_data.config.log_level = config.log_level;
            set_log_level(loaded_data.config.log_level);
            loaded_data.error_log.clear();
            update_state(&mut loaded_data, &state_path);
            loaded_data
        }
        Err(e) => {
            log!(LogLevel::Warn, "No previous state loaded, creating new one");
            log!(LogLevel::Debug, "Error loading previous state: {}", e);
            let mut state = AppState {
                data: String::new(),
                last_updated: current_timestamp(),
                event_counter: 0,
                is_active: false,
                error_log: vec![],
                config: config.clone(),
            };
            state.is_active = false;
            state.data = String::from("Initializing");
            state.config.debug_mode = config.debug_mode;
            state.last_updated = current_timestamp();
            state.config.log_level = config.log_level;
            set_log_level(state.config.log_level);
            state.error_log.clear();
            update_state(&mut state, &state_path);

            state
        }
    };

    // Listening for the sighup
    let reload: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let exit_graceful: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    sighup_watch(reload.clone());
    sigusr_watch(exit_graceful.clone());

    log!(LogLevel::Trace, "Setting state as active...");
    state.is_active = true;
    update_state(&mut state, &state_path);

    if config.debug_mode {
        log!(LogLevel::Info, "Application State: {}", state);
        log!(LogLevel::Info, "Application State: {}", settings);
        log!(LogLevel::Info, "Log Level: {}", config.log_level);
    }

    log!(LogLevel::Info, "{} Started", config.app_name);
    log!(
        LogLevel::Info,
        "Directory Monitoring: {}",
        settings.safe_path()
    );

    // Spawn child process
    log!(LogLevel::Trace, "Running one shot pre child");
    // Run the one-shot process before creating the child
    if let Err(err) = run_one_shot_process(&settings).await {
        log!(LogLevel::Error, "One-shot process failed: {}", err);
        let error = ErrorArrayItem::new(Errors::GeneralError, err);
        log_error(&mut state, error, &state_path);
        return;
    }

    log!(LogLevel::Trace, "Spawning child process...");
    let mut child: SupervisedChild = create_child(&mut state, &state_path, &settings).await;

    match child.clone().await.running().await {
        true => {
            // * safe to call unwrap because we checked that the pid is running
            let xid: u32 = child.clone().await.get_pid().await.unwrap();
            log!(LogLevel::Info, "Child spawned: {}", xid);
            state.data = format!("Child spawned: {}", xid);
            update_state(&mut state, &state_path);
        }
        false => {
            log!(LogLevel::Error, "Failed to spawn child process");
            let error = ErrorArrayItem::new(Errors::GeneralError, "child not spawned".to_string());
            log_error(&mut state, error, &state_path);
            std::process::exit(100);
        }
    }

    let mut change_count: i32 = 0;
    let trigger_count: i32 = settings.changes_needed;

    // Start monitoring the directory and get the asynchronous receiver
    log!(LogLevel::Trace, "Starting directory monitoring...");
    let mut event_rx = match monitor_directory(settings.safe_path()).await {
        Ok(receiver) => {
            log!(LogLevel::Trace, "Successfully started directory monitoring");
            receiver
        }
        Err(err) => {
            log!(LogLevel::Error, "Watcher error: {}", err);
            wind_down_state(&mut state, &state_path);
            std::process::exit(0);
        }
    };

    log!(LogLevel::Trace, "Entering main loop...");
    loop {
        child.monitor.print_usage().await;
        tokio::select! {
            Some(event) = event_rx.recv() => {
                log!(LogLevel::Trace, "Received directory change event: {:?}", event);
                change_count += 1;
                log!(LogLevel::Info, "Change detected: {} out of {}", change_count, trigger_count);
                log!(LogLevel::Debug, "Event details: {:?}", event);

                if change_count >= trigger_count {
                    log!(LogLevel::Info, "Reached {} changes, handling event", trigger_count);
                    state.event_counter += 1;
                    update_state(&mut state, &state_path);
                    log!(LogLevel::Info, "Killing the child");

                    match child.clone().await.kill().await {
                        Ok(_) => {
                            // creating new child
                            child = create_child(&mut state, &state_path, &settings).await;
                            log!(LogLevel::Info, "New child process spawned.");
                        },
                        Err(error) => {
                            log!(LogLevel::Error, "Failed to wait for child process termination: {}", error);
                            log_error(&mut state, error, &state_path);
                        },
                    }

                    change_count = 0; // Reset count
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(3)) => {
                log!(LogLevel::Trace, "Periodic task triggered - checking child process status...");

                if !child.clone().await.running().await {
                    log!(LogLevel::Warn, "Child process {:?} is not running. Restarting...", child.get_pid().await);

                    if let Err(err) = run_one_shot_process(&settings).await {
                        log!(LogLevel::Error, "One-shot process failed: {}", err);
                        let error = ErrorArrayItem::new(Errors::GeneralError, err);
                        log_error(&mut state, error, &state_path);
                        return;
                    }

                    log!(LogLevel::Info, "One shot finished, Spawning new child");

                    child = create_child(&mut state, &state_path, &settings).await;
                    log!(LogLevel::Info, "New child process spawned.");
                }


                // Update state as needed
                state.is_active = true;
                state.data = String::from("Nominal");
                update_state(&mut state, &state_path);
            }
        }

        if reload.load(Ordering::Relaxed) {
            log!(LogLevel::Debug, "Reloading");

            // reload config file
            config = get_config();

            // Updating state data
            state = match StatePersistence::load_state(&state_path) {
                Ok(mut loaded_data) => {
                    log!(LogLevel::Info, "Reloading state data");
                    loaded_data.is_active = true;
                    loaded_data.data = String::from("Reloading");
                    loaded_data.config.debug_mode = config.debug_mode;
                    loaded_data.last_updated = current_timestamp();
                    loaded_data.config.log_level = config.log_level;
                    set_log_level(loaded_data.config.log_level);
                    loaded_data.error_log.clear();
                    update_state(&mut loaded_data, &state_path);
                    loaded_data
                }
                Err(e) => {
                    log!(LogLevel::Warn, "No previous state loaded, creating new one");
                    log!(LogLevel::Debug, "Error loading previous state: {}", e);
                    let mut state = AppState {
                        data: String::new(),
                        last_updated: current_timestamp(),
                        event_counter: 0,
                        is_active: false,
                        error_log: vec![],
                        config: config.clone(),
                    };
                    state.is_active = false;
                    state.data = String::from("Initializing");
                    state.config.debug_mode = config.debug_mode;
                    state.last_updated = current_timestamp();
                    state.config.log_level = config.log_level;
                    set_log_level(state.config.log_level);
                    state.error_log.clear();
                    update_state(&mut state, &state_path);

                    state
                }
            };

            // Killing and redrawing the process
            if let Err(err) = child.kill().await {
                log_error(&mut state, err, &state_path);
                wind_down_state(&mut state, &state_path);
                // We're in a weird state kys and let systemd try again.
                std::process::exit(100)
            }

            // running one shot again
            if let Err(err) = run_one_shot_process(&settings).await {
                log!(LogLevel::Error, "One-shot process failed: {}", err);
                let error = ErrorArrayItem::new(Errors::GeneralError, err);
                log_error(&mut state, error, &state_path);
                return;
            }

            // creating new service
            child = create_child(&mut state, &state_path, &settings).await;
            log!(LogLevel::Info, "New child process spawned.");

            reload.store(false, Ordering::Relaxed);
        }

        if exit_graceful.load(Ordering::Relaxed) {
            log!(LogLevel::Debug, "Exiting gracefully");
            if let Err(err) = child.kill().await {
                log_error(&mut state, err, &state_path);
                wind_down_state(&mut state, &state_path);
                std::process::exit(100)
            }
            std::process::exit(0)
        }
    }
}
