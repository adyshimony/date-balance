// Date Balance - Bitcoin Balance History Checker
// 
// This program allows you to check the historical balance of Bitcoin addresses at any given date.
// It uses the Electrum protocol to connect to a Bitcoin node and calculates balances based on
// confirmed transactions up to the specified timestamp.
// 
// Algorithm:
// 1. For a given address and timestamp:
//    - Convert the address to a script pubkey
//    - Get all transaction history for the address
//    - Filter transactions that were confirmed before the target timestamp
//    - For each transaction:
//      a. Calculate received amount (sum of outputs to the address)
//      b. Calculate spent amount (sum of inputs from the address)
//      c. Net effect = received - spent
//    - Sum all net effects to get the final balance
// 
// The program handles both legacy and native SegWit addresses, and provides detailed
// error handling for various failure scenarios.

use std::time::SystemTime;
use std::collections::HashMap;
use chrono::{DateTime, Utc, NaiveDateTime};
use electrum_client::{Client, ElectrumApi};
use electrum_client::bitcoin::{Txid, Network, Address, Script};
use electrum_client::bitcoin::consensus::Decodable;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use serde_json;
use tiny_http::{Server, Response, StatusCode};

/// Custom error type for handling various failure scenarios in the program
#[derive(Debug)]
enum BalanceError {
    /// Error when connecting to the Electrum server
    ConnectionError(String),
    /// Error when parsing or validating Bitcoin addresses
    InvalidAddress(String),
    /// Error when parsing dates or transaction data
    ParsingError(String),
    /// Error when interacting with the blockchain
    BlockchainError(String),
    /// Error when serializing the response
    SerializationError(String),
}

impl std::error::Error for BalanceError {}
impl std::fmt::Display for BalanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BalanceError::ConnectionError(msg) => write!(f, "Connection error: {}", msg),
            BalanceError::InvalidAddress(msg) => write!(f, "Invalid address: {}", msg),
            BalanceError::ParsingError(msg) => write!(f, "Parsing error: {}", msg),
            BalanceError::BlockchainError(msg) => write!(f, "Blockchain error: {}", msg),
            BalanceError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl From<electrum_client::Error> for BalanceError {
    fn from(err: electrum_client::Error) -> Self {
        BalanceError::ConnectionError(err.to_string())
    }
}

impl From<electrum_client::bitcoin::address::ParseError> for BalanceError {
    fn from(err: electrum_client::bitcoin::address::ParseError) -> Self {
        BalanceError::InvalidAddress(err.to_string())
    }
}

impl From<serde_json::Error> for BalanceError {
    fn from(err: serde_json::Error) -> Self {
        BalanceError::SerializationError(err.to_string())
    }
}

/// Configuration struct for the Electrum client
#[derive(Clone)]
struct Config {
    /// Hostname of the Electrum server
    electrum_host: String,
    /// Port number of the Electrum server
    electrum_port: u16,
    /// Bitcoin network to connect to (mainnet, testnet, etc.)
    network: Network,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            electrum_host: "umbrel.local".to_string(),
            electrum_port: 50002,
            network: Network::Bitcoin,
        }
    }
}

/// Connector to the Electrum server
struct ElectrumConnector {
    /// Electrum client instance
    client: Client,
    /// Configuration settings
    config: Config,
}

impl ElectrumConnector {
    /// Creates a new Electrum connector with the given configuration
    fn new(config: &Config) -> Result<Self, BalanceError> {
        let client = Client::new(&format!("{}:{}", config.electrum_host, config.electrum_port))?;
        Ok(ElectrumConnector {
            client,
            config: config.clone(),
        })
    }
}

/// Main balance checking functionality
struct BalanceChecker {
    /// Electrum connector instance
    connector: ElectrumConnector,
}

impl BalanceChecker {
    /// Creates a new balance checker with the given connector
    fn new(connector: ElectrumConnector) -> Self {
        BalanceChecker { connector }
    }

    /// Checks the balance of a Bitcoin address at a specific timestamp
    fn check_balance(&self, address: &str, timestamp: u64) -> Result<f64, BalanceError> {
        // Parse address and get script
        let addr = Address::from_str(address)?;
        
        // Convert to the right network and get script pubkey
        let addr = addr.require_network(self.connector.config.network)
            .map_err(|_| BalanceError::InvalidAddress("Invalid network".to_string()))?;
        let script = addr.script_pubkey();

        let day_end = timestamp + 24 * 60 * 60 - 1;

        self.connector.client
            .script_get_history(&script)?
            .into_iter()
            .filter(|tx| tx.height > 0)
            .map(|tx| {
                let block_time = self.get_block_timestamp(tx.height as usize)?;
                if block_time <= day_end {
                    self.calculate_tx_effect(&tx.tx_hash, &script)
                } else {
                    Ok(0.0)
                }
            })
            .sum()
    }

    /// Gets the block timestamp for a given block height
    fn get_block_timestamp(&self, height: usize) -> Result<u64, BalanceError> {
        let header = self.connector.client.block_header_raw(height)?;
        let block = electrum_client::bitcoin::block::Header::consensus_decode(&mut &header[..])
            .map_err(|e| BalanceError::ParsingError(e.to_string()))?;
        Ok(block.time as u64)
    }

    /// Calculates the net effect of a transaction on an address's balance
    fn calculate_tx_effect(&self, tx_hash: &Txid, script: &Script) -> Result<f64, BalanceError> {
        let tx = self.connector.client.transaction_get(tx_hash)?;

        let received: f64 = tx.output.iter()
            .filter(|out| out.script_pubkey == *script)
            .map(|out| out.value.to_sat() as f64 / 100_000_000.0)
            .sum();

        let spent: f64 = tx.input.iter()
            .filter_map(|input| {
                let prev_tx = self.connector.client.transaction_get(&input.previous_output.txid).ok()?;
                prev_tx.output.get(input.previous_output.vout as usize)
                    .filter(|out| out.script_pubkey == *script)
                    .map(|out| out.value.to_sat() as f64 / 100_000_000.0)
            })
            .sum();

        Ok(received - spent)
    }

    /// Gets transaction history for an address
    fn get_transaction_history(&self, address: &str) -> Result<Vec<(Txid, u64)>, BalanceError> {
        // Parse address and get script
        let addr = Address::from_str(address)?;
        
        // Convert to the right network and get script pubkey
        let addr = addr.require_network(self.connector.config.network)
            .map_err(|_| BalanceError::InvalidAddress("Invalid network".to_string()))?;
        let script = addr.script_pubkey();
        
        let history = self.connector.client.script_get_history(&script)?;
        Ok(history.into_iter()
            .filter(|tx| tx.height > 0)
            .map(|tx| (tx.tx_hash, tx.height as u64))
            .collect())
    }
    
    /// Gets the full balance information for an address at a specific timestamp
    fn get_balance_info(&self, address: &str, timestamp: u64) -> Result<BalanceResponse, BalanceError> {
        // Get transaction history
        let history = self.get_transaction_history(address)?;
        let totaltx = history.len() as u64;

        // Calculate balance
        let balance = self.check_balance(address, timestamp)?;

        // Find first transaction
        let firsttx = history.iter()
            .min_by_key(|(_, height)| height)
            .map(|(txid, _)| txid.to_string());

        // Initialize lasttx to be the same as firsttx as a fallback
        let mut lasttx = firsttx.clone();
        
        // Try to find the actual last transaction before the target timestamp
        if !history.is_empty() {
            let mut latest_time = 0;
            let mut latest_tx = None;
            
            for (txid, height) in &history {
                match self.get_block_timestamp(*height as usize) {
                    Ok(block_time) => {
                        if block_time <= timestamp && block_time >= latest_time {
                            latest_time = block_time;
                            latest_tx = Some(txid.to_string());
                        }
                    },
                    Err(_) => continue
                }
            }
            
            // Only update lasttx if we found a valid transaction
            if let Some(tx) = latest_tx {
                lasttx = Some(tx);
            }
        }

        // Find the most recent transaction up to today
        let current_timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let mut latest_time = 0;
        let mut lasttx_to_date = None;
        
        for (txid, height) in &history {
            match self.get_block_timestamp(*height as usize) {
                Ok(block_time) => {
                    if block_time <= current_timestamp && block_time >= latest_time {
                        latest_time = block_time;
                        lasttx_to_date = Some(txid.to_string());
                    }
                },
                Err(_) => continue
            }
        }
        
        // Create response
        Ok(BalanceResponse {
            balance,
            totaltx,
            usd: 0.0, // Placeholder for USD value
            firsttx,
            lasttx,
            lasttx_to_date,
        })
    }
}

/// Converts a date string to a Unix timestamp
fn date_to_timestamp(date_str: &str) -> Result<u64, BalanceError> {
    let naive_datetime = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| BalanceError::ParsingError(e.to_string()))?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive_datetime, Utc).timestamp() as u64)
}

#[derive(Serialize, Deserialize)]
struct BalanceResponse {
    balance: f64,
    totaltx: u64,
    usd: f64,
    firsttx: Option<String>,
    lasttx: Option<String>,
    lasttx_to_date: Option<String>,
}

// API request struct
#[derive(Deserialize)]
struct ApiRequest {
    address: String,
    date: String,
}

// Start the HTTP API server
fn start_api_server(config: Config) -> Result<(), BalanceError> {
    let connector = ElectrumConnector::new(&config)?;
    let checker = BalanceChecker::new(connector);
    
    let server = Server::http("127.0.0.1:3030").map_err(|e| BalanceError::ConnectionError(format!("Failed to start server: {}", e)))?;
    println!("API server running at http://127.0.0.1:3030");
    println!("Press Ctrl-C to stop the server");

    for mut request in server.incoming_requests() {
        // Extract query parameters
        let url = request.url();
        
        // Simple solution: Expect url in the format "/balance?address=<address>&date=<date>"
        if !url.starts_with("/balance") {
            let response = Response::from_string("Only /balance endpoint is supported")
                .with_status_code(StatusCode(404));
            let _ = request.respond(response);
            continue;
        }
        
        // Parse query parameters
        let parts: Vec<&str> = url.split('?').collect();
        if parts.len() != 2 {
            let response = Response::from_string("Invalid query parameters")
                .with_status_code(StatusCode(400));
            let _ = request.respond(response);
            continue;
        }
        
        let query = parts[1];
        let mut address = None;
        let mut date = None;
        
        for param in query.split('&') {
            let kv: Vec<&str> = param.split('=').collect();
            if kv.len() != 2 {
                continue;
            }
            
            match kv[0] {
                "address" => address = Some(kv[1].to_string()),
                "date" => date = Some(kv[1].to_string().replace("%20", " ")),
                _ => {}
            }
        }
        
        if address.is_none() || date.is_none() {
            let response = Response::from_string("Missing required parameters: address, date")
                .with_status_code(StatusCode(400));
            let _ = request.respond(response);
            continue;
        }
        
        let address = address.unwrap();
        let date = date.unwrap();
        
        // Convert date to timestamp
        let timestamp = match date_to_timestamp(&date) {
            Ok(ts) => ts,
            Err(e) => {
                let response = Response::from_string(format!("Invalid date format: {}", e))
                    .with_status_code(StatusCode(400));
                let _ = request.respond(response);
                continue;
            }
        };

        // Get full balance information
        match checker.get_balance_info(&address, timestamp) {
            Ok(response_data) => {
                // Convert to JSON and respond
                match serde_json::to_string(&response_data) {
                    Ok(json) => {
                        let response = Response::from_string(json)
                            .with_header(tiny_http::Header {
                                field: "Content-Type".parse().unwrap(),
                                value: "application/json".parse().unwrap(),
                            })
                            .with_status_code(StatusCode(200));
                        let _ = request.respond(response);
                    },
                    Err(e) => {
                        let response = Response::from_string(format!("Error serializing response: {}", e))
                            .with_status_code(StatusCode(500));
                        let _ = request.respond(response);
                    }
                }
            },
            Err(e) => {
                let response = Response::from_string(format!("Error processing request: {}", e))
                    .with_status_code(StatusCode(500));
                let _ = request.respond(response);
            }
        }
    }

    Ok(())
}

/// Main entry point of the program
fn main() -> Result<(), BalanceError> {
    let config = Config::default();
    
    // Start the API server if requested
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--server" {
        return start_api_server(config);
    }
    
    // Otherwise run in CLI mode
    let connector = ElectrumConnector::new(&config)?;
    let checker = BalanceChecker::new(connector);

    let address = "1BBZggkhPgb9Jy4f1fq2itQUKVYq9mMufY";
    let date = "2021-01-07 00:00:00";

    let timestamp = date_to_timestamp(date)?;
    
    // Get full balance info
    let response = checker.get_balance_info(address, timestamp)?;

    // Print JSON response
    let json = serde_json::to_string_pretty(&response)?;
    println!("{}", json);

    Ok(())
}

/// Checks balances for multiple addresses at a specific timestamp
fn check_multiple_balances(checker: &BalanceChecker, addresses: &[String], timestamp: u64) -> Result<HashMap<String, f64>, BalanceError> {
    addresses.iter()
        .map(|addr| {
            checker.check_balance(addr, timestamp)
                .map(|bal| (addr.clone(), bal))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_parsing() {
        let timestamp = date_to_timestamp("2021-07-25 00:00:00").unwrap();
        assert_eq!(timestamp, 1624579200);
    }
}