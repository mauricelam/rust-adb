use clap::Parser;
use mock_adbd::handle_connection;
use tokio::net::TcpListener;

#[derive(Parser, Debug)]
#[command(author, version, about = "Mock adbd server for testing")]
struct Args {
    /// Address to listen on (e.g. 127.0.0.1:0)
    #[arg(long, default_value = "127.0.0.1:0")]
    listen: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let listener = TcpListener::bind(&args.listen).await?;
    let bound_addr = listener.local_addr()?;

    // Print bound address on the very first line of stdout
    println!("{}", bound_addr);

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(async move {
            let _ = handle_connection(stream).await;
        });
    }

    Ok(())
}
