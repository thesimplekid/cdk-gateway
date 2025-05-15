use std::str::FromStr;

use cdk::mint_url::MintUrl;
use cdk::wallet::{MultiMintWallet, WalletBuilder};
use cdk_gateway::config::Settings;
use cdk_gateway::gateway_server::CdkGateway;
use cdk_redb::WalletRedbDatabase;
use std::sync::Arc;

const DEFAULT_WORK_DIR: &str = ".cdk-gateway";

fn main() -> anyhow::Result<()> {
    // Get home directory
    let home_dir = home::home_dir().unwrap();
    let work_dir = home_dir.join(DEFAULT_WORK_DIR);
    
    // Create work directory if it doesn't exist
    if !work_dir.exists() {
        std::fs::create_dir_all(&work_dir)?;
    }
    
    // Create default config file if it doesn't exist
    let config_path = work_dir.join("config.toml");
    if !config_path.exists() {
        println!("Creating default configuration at: {:?}", config_path);
        std::fs::write(
            &config_path,
            r#"# CDK Gateway Configuration

[grpc_processor]
addr = "127.0.0.1"
port = 50051

[wallet]
mnemonic_seed = ""
mint_urls = ["https://mint.example.com"]

[server]
listen_addr = "127.0.0.1"
port = 3000
"#,
        )?;
    }
    
    // Load configuration from the work directory
    let settings = Settings::with_work_dir(Some(work_dir.to_str().unwrap()))?;
    println!("Loaded configuration: {:?}", settings);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let runtime = Arc::new(runtime);

    // Pass settings to your application components here
    let _: anyhow::Result<()> = runtime.block_on(async {
        // Extract settings for each component
        let grpc_settings = settings.grpc_processor;
        let wallet_settings = settings.wallet;
        let server_settings = settings.server;
        
        // Verify that a mnemonic seed is provided
        if wallet_settings.mnemonic_seed.is_empty() {
            return Err(anyhow::anyhow!(
                "Error: No mnemonic seed provided in configuration. Please add a mnemonic_seed to your config.toml file."
            ));
        }

        // Initialize the payment processor
        let payment_processor = cdk_payment_processor::PaymentProcessorClient::new(
            &grpc_settings.addr,
            grpc_settings.port,
            grpc_settings.tls_dir,
        )
        .await?;

        // Make sure the work directory exists
        if !work_dir.exists() {
            std::fs::create_dir_all(&work_dir)?;
        }

        // Parse the mnemonic
        let mnemonic = bip39::Mnemonic::from_str(&wallet_settings.mnemonic_seed)?;

        // Set up the database in the work directory
        let redb_path = work_dir.join("cdk-gateway.redb");
        let localstore = Arc::new(WalletRedbDatabase::new(&redb_path)?);

        let mut wallets = vec![];

        let seed = mnemonic.to_seed_normalized("");

        for mint_url in wallet_settings.mint_urls.iter() {
            let builder = WalletBuilder::new()
                .mint_url(MintUrl::from_str(mint_url)?)
                .unit(cdk::nuts::CurrencyUnit::Sat)
                .localstore(localstore.clone())
                .seed(&seed);

            let wallet = builder.build()?;

            let wallet_clone = wallet.clone();

            tokio::spawn(async move {
                if let Err(err) = wallet_clone.get_mint_info().await {
                    tracing::error!(
                        "Could not get mint quote for {}, {}",
                        wallet_clone.mint_url,
                        err
                    );
                }
            });

            wallets.push(wallet);
        }

        let multi_mint_wallet = MultiMintWallet::new(localstore, Arc::new(seed), wallets);

        // Start the gateway server with all components
        let gateway = CdkGateway::new(Arc::new(payment_processor), multi_mint_wallet);

        // Create socket address from server settings
        let socket_addr = std::net::SocketAddr::new(
            std::net::IpAddr::from_str(&server_settings.listen_addr)?,
            server_settings.port,
        );

        // Start the server
        gateway
            .start_server(socket_addr, wallet_settings.mint_urls.clone())
            .await
    });

    Ok(())
}
