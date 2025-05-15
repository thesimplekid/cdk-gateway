use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, extract::State};
use cdk::Bolt11Invoice;
use cdk::amount::Amount;
use cdk::cdk_payment::{self, Bolt11OutgoingPaymentOptions, MintPayment, OutgoingPaymentOptions};
use cdk::mint_url::MintUrl;
use cdk::nuts::nut18::PaymentRequestBuilder;
use cdk::nuts::{CurrencyUnit, Nut10Secret, SpendingConditions, Token};
use cdk::util::unix_time;
use cdk::wallet::types::WalletKey;
use cdk::wallet::{MultiMintWallet, ReceiveOptions, SendOptions};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// Cashu Lsp State
#[derive(Clone)]
pub struct CdkGateway {
    node: Arc<dyn MintPayment<Err = cdk_payment::Error> + Send + Sync>,
    wallets: MultiMintWallet,
    server_cancel: CancellationToken,
}

impl CdkGateway {
    /// Create a new CdkGateway instance
    pub fn new(
        node: Arc<dyn MintPayment<Err = cdk_payment::Error> + Send + Sync>,
        wallets: MultiMintWallet,
    ) -> Self {
        Self {
            node,
            wallets,
            server_cancel: CancellationToken::new(),
        }
    }

    /// Get a reference to the payment node
    pub fn node(&self) -> &Arc<dyn MintPayment<Err = cdk_payment::Error> + Send + Sync> {
        &self.node
    }

    /// Get a reference to the wallet collection
    pub fn wallets(&self) -> &MultiMintWallet {
        &self.wallets
    }

    /// Start the Axum HTTP server for the gateway API in a background task
    ///
    /// # Arguments
    /// * `self` - The CdkGateway instance
    /// * `bind_address` - The address to bind the server to (e.g. "127.0.0.1:3000")
    /// * `mints` - List of mint URLs that this gateway supports
    ///
    /// # Returns
    /// A ServerHandle that can be used to stop the server
    pub async fn start_server(
        &self,
        bind_address: SocketAddr,
        mints: Vec<MintUrl>,
    ) -> anyhow::Result<()> {
        let gateway = Arc::new(self.clone());

        let cancel = self.server_cancel.clone();

        // Spawn the server task
        let app = create_cashu_lsp_router(gateway, mints).await.unwrap();

        tracing::info!("Starting CDK Gateway server on {}", bind_address);
        let listener = tokio::net::TcpListener::bind(bind_address).await.unwrap();
        // Configure the server to gracefully shut down
        Ok(axum::serve(listener, app)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await?)
    }

    /// Stop the server and cancel all tasks
    pub async fn stop_server(&self) -> anyhow::Result<()> {
        tracing::info!("Shutting down CDK Gateway server");
        self.server_cancel.cancel();
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatwayInfo {
    pub mints: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum PaymentMethod {
    #[default]
    #[serde(rename = "bolt11")]
    Bolt11,
    #[serde(rename = "bolt12")]
    Bolt12,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeltRequest {
    pub method: PaymentMethod,
    pub request: String,
    pub amount: Option<Amount>,
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeltResponse {
    pub payment_proof: String,
    pub change: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: u16,
    pub message: String,
    pub details: Option<String>,
    #[serde(skip)]
    pub payment_request: Option<String>,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        // If the error is about insufficient funds or related to payment, use 402 Payment Required
        let status = if self.code == 400
            && (self.message.contains("Insufficient funds")
                || self.message.contains("Missing amount")
                || self.message.contains("Token verification failed"))
        {
            StatusCode::PAYMENT_REQUIRED
        } else {
            StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        };

        // Copy relevant data for serialization
        let serializable_error = ErrorResponse {
            code: self.code,
            message: self.message.clone(),
            details: self.details.clone(),
            payment_request: None, // Skip this in serialization
        };

        // Create a basic response with the status and JSON body
        let mut response = (status, Json(serializable_error)).into_response();

        // If we're returning a 402 Payment Required, add the X-cashu header
        if status == StatusCode::PAYMENT_REQUIRED {
            if let Some(payment_request) = self.payment_request {
                if let Ok(header_value) = header::HeaderValue::from_str(&payment_request) {
                    response
                        .headers_mut()
                        .insert(header::HeaderName::from_static("x-cashu"), header_value);
                }
            }
        }

        response
    }
}

#[derive(Clone)]
pub struct GatwayState {
    pub inner: Arc<CdkGateway>,
    pub mints: Vec<MintUrl>,
}

pub async fn create_cashu_lsp_router(
    gateway: Arc<CdkGateway>,
    mints: Vec<MintUrl>,
) -> anyhow::Result<Router> {
    tracing::debug!(
        "Creating CDK Gateway router with {} supported mints",
        mints.len()
    );
    let gateway_state = GatwayState {
        inner: gateway,
        mints,
    };
    let router = Router::new()
        .route("/payment", post(post_melt_request))
        .route("/mints", get(get_mints))
        .with_state(gateway_state);

    Ok(router)
}

pub async fn get_mints(
    State(state): State<GatwayState>,
) -> Result<Json<Vec<MintUrl>>, ErrorResponse> {
    tracing::debug!("Request received for /mints endpoint");
    Ok(Json(state.mints))
}

pub async fn post_melt_request(
    State(state): State<GatwayState>,
    Json(payload): Json<MeltRequest>,
) -> Result<Json<MeltResponse>, ErrorResponse> {
    tracing::info!("Payment request received with method: {:?}", payload.method);
    let hash;
    let (amount_to_pay_sat, outgoing_options) = match payload.method {
        PaymentMethod::Bolt11 => {
            let bolt11: Bolt11Invoice = payload.request.parse().map_err(|_| ErrorResponse {
                code: 400,
                message: "Invalid BOLT11 invoice".to_string(),
                details: None,
                payment_request: None,
            })?;

            let amount = if let Some(amount) = bolt11.amount_milli_satoshis() {
                (amount / 1_000).into()
            } else {
                payload.amount.ok_or(ErrorResponse {
                    code: 400,
                    message: "Missing amount".to_string(),
                    details: Some(
                        "Invoice has no amount specified. Please provide an amount in the request."
                            .to_string(),
                    ),
                    payment_request: None,
                })?
            };

            hash = bolt11.payment_hash().to_owned();

            let outgoing = OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                bolt11,
                max_fee_amount: None,
                timeout_secs: None,
                melt_options: None,
            }));

            (amount, outgoing)
        }
        PaymentMethod::Bolt12 => {
            return Err(ErrorResponse {
                code: 400,
                message: "Payment method not supported".to_string(),
                details: Some("BOLT12 payment method is not supported".to_string()),
                payment_request: None,
            });
        }
    };

    let nut10 = SpendingConditions::HTLCConditions {
        data: hash,
        conditions: None,
    };

    // Build the payment request with the correct amount for any error responses
    let payment_request = PaymentRequestBuilder::default()
        .unit(CurrencyUnit::Sat)
        .amount(u64::from(amount_to_pay_sat))
        .mints(state.mints.clone())
        .nut10(nut10.into())
        .build();

    let tokens: Vec<Token> = payload
        .tokens
        .iter()
        .flat_map(|t| Token::from_str(t))
        .collect();

    let token_amount: Vec<Amount> = tokens.iter().map(|a| a.value().unwrap()).collect();
    let total_amount = Amount::try_sum(token_amount).unwrap();

    if total_amount < amount_to_pay_sat {
        tracing::error!("Not enough proofs provided");
        return Err(ErrorResponse {
            code: 402,
            message: "Insufficient funds".to_string(),
            details: Some(format!(
                "Required: {}, provided: {}",
                amount_to_pay_sat, total_amount
            )),
            payment_request: Some(payment_request.to_string()),
        });
    }

    let mut used_mints = vec![];

    for token in tokens.iter() {
        let mint_url = token.mint_url().unwrap();
        let wallet = state
            .inner
            .wallets()
            .get_wallet(&WalletKey::new(mint_url.clone(), CurrencyUnit::Sat))
            .await
            .expect("wallet");

        used_mints.push(mint_url);

        wallet.verify_token_dleq(token).await.map_err(|e| {
            tracing::error!("Invalid dleq: {}", e);
            ErrorResponse {
                code: 400,
                message: "Token verification failed".to_string(),
                details: Some(format!("DLEQ verification error: {}", e)),
                payment_request: Some(payment_request.to_string()),
            }
        })?;

        for proof in token.proofs() {
            let secret: Nut10Secret = proof.secret.try_into().map_err(|err| {
                tracing::error!("Invalid secret: {}", err);
                ErrorResponse {
                    code: 400,
                    message: "Token verification failed".to_string(),
                    details: Some(format!("Secret validation failed: {}", err)),
                    payment_request: Some(payment_request.to_string()),
                }
            })?;

            let secret_spending_conditions: SpendingConditions = secret.try_into().unwrap();

            match secret_spending_conditions {
                SpendingConditions::HTLCConditions { data, conditions } => {
                    if data != hash {
                        tracing::debug!("Payment hash does not equal token hash");
                        return Err(ErrorResponse {
                            code: 400,
                            message: "Token hash does not match payment hash".to_string(),
                            details: None,
                            payment_request: Some(payment_request.to_string()),
                        });
                    }

                    if let Some(conditions) = conditions {
                        if let Some(locktime) = conditions.locktime {
                            if locktime < unix_time() + 900 {
                                tracing::debug!("Token locktime is not long enough");
                                return Err(ErrorResponse {
                                    code: 400,
                                    message: "Token lock time is not long enough".to_string(),
                                    details: None,
                                    payment_request: Some(payment_request.to_string()),
                                });
                            }
                        }
                    }
                }
                SpendingConditions::P2PKConditions {
                    data: _,
                    conditions: _,
                } => {
                    return Err(ErrorResponse {
                        code: 400,
                        message: "Token verification failed".to_string(),
                        details: None,
                        payment_request: Some(payment_request.to_string()),
                    });
                }
            }
        }
    }

    let payment_response = state
        .inner
        .node()
        .make_payment(&CurrencyUnit::Sat, outgoing_options)
        .await
        .map_err(|e| {
            tracing::error!("Payment failed: {}", e);
            ErrorResponse {
                code: 500,
                message: "Payment failed".to_string(),
                details: Some(e.to_string()),
                payment_request: None,
            }
        })?;

    tracing::info!("Payment successfully processed");

    for token in tokens.iter() {
        let wallet = state
            .inner
            .wallets()
            .get_wallet(&WalletKey::new(
                token.mint_url().unwrap(),
                CurrencyUnit::Sat,
            ))
            .await
            .expect("wallet");

        wallet
            .receive(
                &token.to_string(),
                ReceiveOptions {
                    preimages: vec![payment_response.payment_proof.clone().ok_or(
                        ErrorResponse {
                            code: 500,
                            message: "Missing payment proof".to_string(),
                            details: None,
                            payment_request: None,
                        },
                    )?],
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| ErrorResponse {
                code: 500,
                message: "Failed to process token receive".to_string(),
                details: Some(e.to_string()),
                payment_request: None,
            })?;
    }

    let proof = payment_response.payment_proof.ok_or(ErrorResponse {
        code: 500,
        message: "Missing payment proof in response".to_string(),
        details: None,
        payment_request: None,
    })?;

    let change_amount = total_amount
        .checked_sub(payment_response.total_spent)
        .unwrap_or_default();

    tracing::info!("Preparing change payment of {}", change_amount);
    let mut change = vec![];

    for mint_url in used_mints {
        let wallet = state
            .inner
            .wallets()
            .get_wallet(&WalletKey::new(mint_url.clone(), CurrencyUnit::Sat))
            .await
            .expect("wallet");

        let change_prepared_send = wallet
            .prepare_send(change_amount, SendOptions::default())
            .await
            .unwrap();

        let token = wallet.send(change_prepared_send, None).await.unwrap();

        change.push(token.to_string());
    }

    tracing::info!(
        "Payment request completed successfully with {} tokens in change",
        change.len()
    );
    Ok(Json(MeltResponse {
        payment_proof: proof,
        change,
    }))
}
