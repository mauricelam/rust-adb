use clap::Parser;
use mock_adbd::{handle_connection_with_options, MockAdbdOptions};
use tokio::net::TcpListener;

#[derive(Parser, Debug)]
#[command(author, version, about = "Mock adbd server for testing")]
struct Args {
    /// Address to listen on (e.g. 127.0.0.1:0)
    #[arg(long, default_value = "127.0.0.1:0")]
    listen: String,

    /// Disable shell_v2 feature advertisement
    #[arg(long)]
    no_shell_v2: bool,

    /// Disable stat_v2 feature advertisement
    #[arg(long)]
    no_stat_v2: bool,

    /// Disable ls_v2 feature advertisement
    #[arg(long)]
    no_ls_v2: bool,

    /// Disable sendrecv_v2 feature advertisement
    #[arg(long)]
    no_sendrecv_v2: bool,

    /// Enable fixed_push_mkdir feature advertisement
    #[arg(long)]
    fixed_push_mkdir: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let options = MockAdbdOptions {
        enable_shell_v2: !args.no_shell_v2,
        enable_stat_v2: !args.no_stat_v2,
        enable_ls_v2: !args.no_ls_v2,
        enable_sendrecv_v2: !args.no_sendrecv_v2,
        enable_fixed_push_mkdir: args.fixed_push_mkdir,
        ..Default::default()
    };

    let listener = TcpListener::bind(&args.listen).await?;
    let bound_addr = listener.local_addr()?;

    // Print bound address on the very first line of stdout
    println!("{}", bound_addr);

    while let Ok((stream, _)) = listener.accept().await {
        let options = options.clone();
        tokio::spawn(async move {
            let _ = handle_connection_with_options(stream, options).await;
        });
    }

    Ok(())
}
