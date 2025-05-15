use std::str::FromStr;

use cdk::mint_url::MintUrl;
use cdk::wallet::{MultiMintWallet, WalletBuilder};
use cdk_gateway::config::Settings;
use cdk_gateway::gateway_server::CdkGateway;
use cdk_redb::WalletRedbDatabase;
use std::sync::Arc;

const DEFAULT_WORK_DIR: &str = ".cdk-gateway";

fn main() -> anyhow::Result<()> {
    // Load configuration
    let settings = Settings::new()?;
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

        // Initialize the payment processor
        let payment_processor = cdk_payment_processor::PaymentProcessorClient::new(
            &grpc_settings.addr,
            grpc_settings.port,
            grpc_settings.tls_dir,
        )
        .await?;

        let home_dir = home::home_dir().unwrap();
        let work_dir = home_dir.join(DEFAULT_WORK_DIR);

        let mnemonic = bip39::Mnemonic::from_str(&wallet_settings.mnemonic_seed)?;

        let redb_path = work_dir.join("cdk-cli.redb");
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
