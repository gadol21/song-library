//! Sing-Along Studio: runs the server and, by default, shows it in a desktop window.
//!
//!   SingAlongStudio            desktop app (window), data under %APPDATA%\Sing-Along Studio
//!   SingAlongStudio --server   web server only, like run.bat / run.sh (--host 0.0.0.0 --port 8000)

use singalong::{config, server};

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn run_server(host: String, port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("async runtime");
    rt.block_on(async {
        server::startup();
        let listener = match tokio::net::TcpListener::bind((host.as_str(), port)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Cannot listen on {}:{}: {}", host, port, e);
                std::process::exit(1);
            }
        };
        println!("==================================================");
        println!("  Sing-Along Studio | אולפן שירה בציבור וקריוקי");
        println!("  Starting server at http://localhost:{}", port);
        println!("==================================================");
        let _ = server::serve(listener, None).await;
    });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--server") {
        config::init(config::default_home(false));
        let host = arg_value(&args, "--host").unwrap_or_else(|| "0.0.0.0".into());
        let port = arg_value(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(8000);
        run_server(host, port);
        return;
    }
    #[cfg(windows)]
    {
        singalong::desktop::run();
    }
    #[cfg(not(windows))]
    {
        singalong::log::info("No desktop window on this platform; starting the web server.");
        config::init(config::default_home(false));
        run_server("0.0.0.0".into(), 8000);
    }
}
