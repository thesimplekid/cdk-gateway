# CDK Gateway

CDK Gateway is a service that bridges Cashu ecash tokens with Lightning Network payments. It allows users to spend their Cashu tokens to pay Lightning invoices through a simple HTTP API.

## What is Cashu?

Cashu is a privacy-focused ecash protocol for Bitcoin. It allows users to hold and transfer value with tokens that provide strong privacy guarantees. To learn more about Cashu, visit [the Cashu website](https://cashu.space).

## Features

- **Token Handling**: Process Cashu tokens for payments
- **Lightning Integration**: Make lightning payments using Cashu tokens
- **Multi-Mint Support**: Connect to multiple Cashu mints simultaneously
- **RESTful API**: Simple HTTP API for client integration

## Installation

### Prerequisites

- Rust and Cargo
- GRPC processor service for lightning payments

### Building from Source

```sh
git clone https://github.com/yourusername/cdk-gateway.git
cd cdk-gateway
cargo build --release
```

The compiled binary will be available at `target/release/cdk_gateway`.

## Configuration

CDK Gateway provides an example configuration file (`config.example.toml`) that you can use as a starting point.
Copy this example to `config.toml` and adjust as needed for your environment:

```sh
cp config.example.toml config.toml
# Edit config.toml as needed
```

CDK Gateway uses a flexible configuration system that supports both configuration files and environment variables.

### Configuration File

By default, the application will look for a `config.toml` file in the current working directory. You can also specify a custom configuration path using the `CDK_GATEWAY_CONFIG` environment variable.

### Environment Variables

Configuration can also be provided via environment variables. The environment variables should be prefixed with `CDK_GATEWAY__` and use double underscores (`__`) as separators for nested properties.

For example:

```sh
# Configure the gRPC processor address and port
export CDK_GATEWAY__GRPC_PROCESSOR__ADDR=127.0.0.1
export CDK_GATEWAY__GRPC_PROCESSOR__PORT=50051

# Configure wallet settings
export CDK_GATEWAY__WALLET__MNEMONIC_SEED="your twelve word mnemonic seed phrase goes here"
export CDK_GATEWAY__WALLET__MINT_URLS="[\"https://mint1.example.com\"]"

# Configure server settings
export CDK_GATEWAY__SERVER__LISTEN_ADDR=0.0.0.0
export CDK_GATEWAY__SERVER__PORT=3000
```

Note that for array values like `MINT_URLS`, you need to provide a properly escaped JSON array string.

### Configuration Precedence

Configuration values are loaded in the following order, with later sources overriding earlier ones:

1. Default values
2. Configuration file
3. Environment variables

This allows for flexible configuration in different deployment environments.

## Wallet Configuration

The wallet configuration section allows you to set up the following:

- **mnemonic_seed**: An optional BIP39 mnemonic seed phrase. If not provided, a new one will be generated.
- **mint_urls**: A list of mint URLs to connect to. These are the Cashu mints that the gateway will interact with.

Example wallet configuration in TOML:

```toml
[wallet]
mnemonic_seed = "your twelve word mnemonic seed phrase goes here"
mint_urls = ["https://mint1.example.com", "https://mint2.example.com"]
```

## Server Configuration

The server configuration section controls the Axum HTTP server settings:

- **listen_addr**: The IP address the server should listen on. Use "127.0.0.1" for local access only, or "0.0.0.0" to accept connections from any IP address.
- **port**: The TCP port the server should listen on.

Example server configuration in TOML:

```toml
[server]
listen_addr = "0.0.0.0"  # Listen on all network interfaces
port = 3000              # Listen on port 3000
```

## Usage

### Starting the Gateway

```sh
./target/release/cdk_gateway
```

The server will start and listen on the configured address and port (default: 127.0.0.1:3000).

### API Endpoints

The CDK Gateway exposes the following HTTP API endpoints:

#### Get Supported Mints

Retrieve the list of supported Cashu mints.

```sh
curl -X GET http://localhost:3000/mints
```

Example response:

```json
[
  "https://mint1.example.com",
  "https://mint2.example.com"
]
```

#### Process Payment

Make a lightning payment using Cashu tokens.

```sh
curl -X POST http://localhost:3000/payment \
  -H "Content-Type: application/json" \
  -d '{
    "method": "bolt11",
    "request": "lnbc100n1p3x...",
    "tokens": [
      "cashuB..."
    ]
  }'
```

Note: The token format shown above is simplified. In practice, you'll need to provide valid Cashu tokens that conform to the Cashu protocol specification.

Example response:

```json
{
  "payment_proof": "022222f...",
  "change": [
    "cashuB..."
  ]
}
```

## Request Format

### Payment Request

| Field | Type | Description |
|-------|------|-------------|
| `method` | String | Payment method: "bolt11" |
| `request` | String | BOLT11 lightning invoice |
| `amount` | Number (optional) | Payment amount (if not specified in invoice) |
| `tokens` | Array | Array of Cashu Token objects |

The tokens must be valid Cashu tokens with correct proofs that match the lightning payment hash.

### Response Format

| Field | Type | Description |
|-------|------|-------------|
| `payment_proof` | String | Proof of payment |
| `change` | Array | Array of Cashu tokens for change (if any) |

## Working with Cashu Tokens

Cashu tokens have a specific structure required by the protocol. Here's a more detailed example of a payment request with a properly formatted token:

```json
{
  "method": "bolt11",
  "request": "lnbc100n1p3x...",
  "tokens": [
    {
      "mint": "https://mint.example.com",
      "proofs": [
        {
          "id": "b68c7d51",
          "amount": 10,
          "secret": "b79a4dbc164ceef2ea6ad15c055e9fbc2955528bfa22ce856a3fb9570800a7"  
        }
      ],
      "signatures": [
        {
          "id": "b68c7d51",
          "signature": "91f5763a238cf8d9b771b1a42d40a0983a8ba086bed8adc8e67c637ef1e6a87adb7a2a9ddc5e175c874dc0542158d752b3a0ad665748d5c5b7391e2a07c7721"
        }
      ]
    }
  ]
}
```

Note: Cashu tokens involve cryptographic operations, and this example is simplified. Consult the Cashu protocol documentation for details on generating valid tokens.

## Error Handling

The API returns appropriate HTTP status codes along with error messages:

```json
{
  "code": 400,
  "message": "Invalid BOLT11 invoice",
  "details": null
}
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the LICENSE file for details.
