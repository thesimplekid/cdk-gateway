use std::str::FromStr;

use cdk::mint_url::MintUrl;
use cdk::wallet::{MultiMintWallet, WalletBuilder};
use cdk_gateway::config::Settings;
use cdk_gateway::gateway_server::CdkGateway;
use cdk_redb::WalletRedbDatabase;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const DEFAULT_WORK_DIR: &str = ".cdk-gateway";

fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // Default to INFO level if RUST_LOG environment variable is not set
            "cdk_gateway=info,tower_http=debug,axum::rejection=trace".into()
        }))
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();

    tracing::info!("Starting CDK Gateway");
    // Get home directory
    let home_dir = home::home_dir().unwrap();
    let work_dir = home_dir.join(DEFAULT_WORK_DIR);
    
    
    // Load configuration from the work directory
    let settings = Settings::with_work_dir(Some(work_dir.to_str().unwrap()))?;
    tracing::info!("Loaded configuration");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let runtime = Arc::new(runtime);

    // Pass settings to your application components here
    let gateway_result: anyhow::Result<CdkGateway> = runtime.block_on(async {
        tracing::info!("Initializing application components");
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
        tracing::info!("Connecting to payment processor at {}:{}", grpc_settings.addr, grpc_settings.port);
        let payment_processor = cdk_payment_processor::PaymentProcessorClient::new(
            &grpc_settings.addr,
            grpc_settings.port,
            grpc_settings.tls_dir,
        )
        .await?;
        tracing::info!("Payment processor connection established");

        // Make sure the work directory exists
        if !work_dir.exists() {
            tracing::info!("Creating work directory at {:?}", work_dir);
            std::fs::create_dir_all(&work_dir)?;
        }

        // Parse the mnemonic
        tracing::debug!("Initializing wallet from mnemonic seed");
        let mnemonic = bip39::Mnemonic::from_str(&wallet_settings.mnemonic_seed)?;

        // Set up the database in the work directory
        let redb_path = work_dir.join("cdk-gateway.redb");
        tracing::info!("Opening database at {:?}", redb_path);
        let localstore = Arc::new(WalletRedbDatabase::new(&redb_path)?);

        let mut wallets = vec![];

        let seed = mnemonic.to_seed_normalized("");
        tracing::info!("Initializing wallets for {} mint URLs", wallet_settings.mint_urls.len());

        for mint_url in wallet_settings.mint_urls.iter() {
            tracing::info!("Setting up wallet for mint: {}", mint_url);
            let builder = WalletBuilder::new()
                .mint_url(MintUrl::from_str(mint_url)?)
                .unit(cdk::nuts::CurrencyUnit::Sat)
                .localstore(localstore.clone())
                .seed(&seed);

            let wallet = builder.build()?;

            let wallet_clone = wallet.clone();

            tokio::spawn(async move {
                tracing::debug!("Fetching mint info for {}", wallet_clone.mint_url);
                if let Err(err) = wallet_clone.get_mint_info().await {
                    tracing::error!(
                        "Could not get mint quote for {}, {}",
                        wallet_clone.mint_url,
                        err
                    );
                } else {
                    tracing::debug!("Successfully retrieved mint info for {}", wallet_clone.mint_url);
                }
            });

            wallets.push(wallet);
        }

        let multi_mint_wallet = MultiMintWallet::new(localstore, Arc::new(seed), wallets);
        tracing::info!("Multi-mint wallet initialized");

        // Start the gateway server with all components
        let gateway = CdkGateway::new(Arc::new(payment_processor), multi_mint_wallet);

        // Create socket address from server settings
        let socket_addr = std::net::SocketAddr::new(
            std::net::IpAddr::from_str(&server_settings.listen_addr)?,
            server_settings.port,
        );

        tracing::info!("Starting server on {}", socket_addr);

        // Create a new gateway instance to return
        let gateway_clone = gateway.clone();

        // Start the server in a separate task
        tokio::spawn(async move {
            if let Err(e) = gateway.start_server(socket_addr, wallet_settings.mint_urls.clone()).await {
                tracing::error!("Server error: {}", e);
            }
        });

        Ok(gateway_clone)
    });

    // Handle the result of gateway initialization
    let gateway = match gateway_result {
        Ok(gateway) => gateway,
        Err(e) => {
            tracing::error!("Failed to initialize gateway: {}", e);
            return Err(e);
        }
    };

    // Set up signal handling for graceful shutdown
    let gateway_for_shutdown = gateway.clone();
    let runtime_for_shutdown = runtime.clone();
    
    // Create a channel to signal when shutdown is complete
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

    // Common shutdown function
    let create_shutdown_handler = |tx: std::sync::mpsc::Sender<()>, gw: CdkGateway, rt: Arc<tokio::runtime::Runtime>| {
        move || {
            tracing::info!("Received shutdown signal, shutting down...");
            let gateway = gw.clone();
            let runtime = rt.clone();
            let shutdown_tx = tx.clone();
            
            // Shutdown the gateway
            runtime.block_on(async {
                if let Err(e) = gateway.stop_server().await {
                    tracing::error!("Error during shutdown: {}", e);
                }
                // Signal that shutdown is complete
                let _ = shutdown_tx.send(());
            });
        }
    };

    // Set up SIGINT (Ctrl+C) handler
    let sigint_handler = create_shutdown_handler(
        shutdown_tx.clone(),
        gateway_for_shutdown.clone(),
        runtime_for_shutdown.clone()
    );
    ctrlc::set_handler(sigint_handler).expect("Error setting Ctrl-C handler");

    tracing::info!("CDK Gateway running. Press Ctrl+C to stop.");

    // Wait for shutdown signal
    let _ = shutdown_rx.recv();
    tracing::info!("CDK Gateway shutdown complete");
    
    Ok(())
}
