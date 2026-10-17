use adb_proxy::{Args, ProxyServer};
use clap::Parser;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let server = match ProxyServer::bind(&args.listen, &args.target, &args.out).await {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Failed to bind listener on {}: {}", args.listen, err);
            return ExitCode::FAILURE;
        }
    };

    if let Ok(addr) = server.local_addr() {
        println!("Listening on {}", addr);
    }

    if let Err(err) = server
        .run_until(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    {
        eprintln!("Proxy server error: {}", err);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
