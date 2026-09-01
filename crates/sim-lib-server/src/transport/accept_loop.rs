fn run_accept_loop(runtime: Arc<ServerRuntime>, site: Arc<dyn EvalSite>) {
    while !runtime.is_stopping() {
        let accepted = match runtime.accept_timeout(Duration::from_millis(25)) {
            Ok(connection) => connection,
            Err(_) => break,
        };
        let Some(mut connection) = accepted else {
            thread::sleep(Duration::from_millis(25));
            continue;
        };
        match runtime.thread_mode() {
            ThreadMode::Main | ThreadMode::Coop => {
                let _ = connection.serve_connection(&runtime, &site);
            }
            ThreadMode::Spawn => {
                let runtime_for_worker = runtime.clone();
                let site_for_worker = site.clone();
                let handle = thread::spawn(move || {
                    let _ = connection.serve_connection(&runtime_for_worker, &site_for_worker);
                });
                if runtime.register_worker_thread(handle).is_err() {
                    runtime.begin_stop();
                    break;
                }
            }
            ThreadMode::Pool => {
                let runtime_for_worker = runtime.clone();
                let site_for_worker = site.clone();
                default_worker_pool().execute(move || {
                    let _ = connection.serve_connection(&runtime_for_worker, &site_for_worker);
                });
            }
            ThreadMode::Coroutine(_) => {}
        }
    }
}
