use std::time::SystemTime;
use std::collections::HashMap;
use chrono::{DateTime, Utc, NaiveDateTime};
use electrum_client::{Client, ElectrumApi};
use electrum_client::bitcoin::{Txid, Network, Address, Script};
use electrum_client::bitcoin::consensus::Decodable;
use std::str::FromStr;

// Custom error type
#[derive(Debug)]
enum BalanceError {
    ConnectionError(String),
    InvalidAddress(String),
    ParsingError(String),
    BlockchainError(String),
}

impl std::error::Error for BalanceError {}
impl std::fmt::Display for BalanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BalanceError::ConnectionError(msg) => write!(f, "Connection error: {}", msg),
            BalanceError::InvalidAddress(msg) => write!(f, "Invalid address: {}", msg),
            BalanceError::ParsingError(msg) => write!(f, "Parsing error: {}", msg),
            BalanceError::BlockchainError(msg) => write!(f, "Blockchain error: {}", msg),
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

// Configuration struct
#[derive(Clone)]
struct Config {
    electrum_host: String,
    electrum_port: u16,
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

// Electrum connector
struct ElectrumConnector {
    client: Client,
    config: Config,
}

impl ElectrumConnector {
    fn new(config: &Config) -> Result<Self, BalanceError> {
        let client = Client::new(&format!("{}:{}", config.electrum_host, config.electrum_port))?;
        Ok(ElectrumConnector {
            client,
            config: config.clone(),
        })
    }
}

// Balance checker
struct BalanceChecker {
    connector: ElectrumConnector,
}

impl BalanceChecker {
    fn new(connector: ElectrumConnector) -> Self {
        BalanceChecker { connector }
    }

    fn check_balance(&self, address: &str, timestamp: u64) -> Result<f64, BalanceError> {
        let script = Address::from_str(address)
            .map_err(|e| BalanceError::InvalidAddress(e.to_string()))?
            .require_network(self.connector.config.network)
            .map_err(|e| BalanceError::InvalidAddress(e.to_string()))?
            .script_pubkey();

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

    fn get_block_timestamp(&self, height: usize) -> Result<u64, BalanceError> {
        let header = self.connector.client.block_header_raw(height)?;
        let block = electrum_client::bitcoin::block::Header::consensus_decode(&mut &header[..])
            .map_err(|e| BalanceError::ParsingError(e.to_string()))?;
        Ok(block.time as u64)
    }

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
}

// Utility functions
fn date_to_timestamp(date_str: &str) -> Result<u64, BalanceError> {
    let naive_datetime = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| BalanceError::ParsingError(e.to_string()))?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive_datetime, Utc).timestamp() as u64)
}

fn main() -> Result<(), BalanceError> {
    let config = Config::default();
    let connector = ElectrumConnector::new(&config)?;
    let checker = BalanceChecker::new(connector);

    let address = "bc1qv0gtzvsjh5tpmeehyp90dn6k8w6ddlfdtqej9c";
    let date = "2021-06-25 00:00:00";

    let start = SystemTime::now();
    let timestamp = date_to_timestamp(date)?;
    let balance = checker.check_balance(address, timestamp)?;

    println!(
        "Balance of {} at {} was: {} BTC (took {:.2}s)",
        address,
        date,
        balance,
        start.elapsed()
            .map_err(|e| BalanceError::ParsingError(e.to_string()))?
            .as_secs_f64()
    );

    Ok(())
}

// Example additional functionality for multiple addresses
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
        let timestamp = date_to_timestamp("2021-06-25 00:00:00").unwrap();
        assert_eq!(timestamp, 1624579200);
    }
}