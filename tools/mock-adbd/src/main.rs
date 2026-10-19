use clap::Parser;
use mock_adbd::MockAdbdServer;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address to listen on
    #[arg(long, default_value = "127.0.0.1:0")]
    listen: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let server = MockAdbdServer::bind(&args.listen).await?;
    println!("{}", server.local_addr());
    server.run().await?;
    Ok(())
}
