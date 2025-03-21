use std::collections::HashMap;
use chrono::{DateTime, Utc};
use electrum_client::{Client, ElectrumApi};
use electrum_client::bitcoin::{Txid, Transaction};
use std::str::FromStr;
use hex;
use electrum_client::bitcoin::Address;
use electrum_client::bitcoin::hashes::{Hash, HashEngine};
use electrum_client::bitcoin::hashes::sha256::Hash as Sha256;
use std::time::SystemTime;
use chrono::NaiveDateTime;
use electrum_client::bitcoin::consensus::Decodable;


// Function to convert address to scripthash
fn address_to_scripthash(address_str: &str) -> Result<String, String> {
    // Parse the address string into a Bitcoin Address
    let address = match Address::from_str(address_str) {
        Ok(addr) => addr,
        Err(e) => return Err(format!("Invalid address: {}", e)),
    };
    
    // Get the script pubkey for this address
    let script = address.require_network(electrum_client::bitcoin::Network::Bitcoin)
        .map_err(|e| format!("Network error: {}", e))?
        .script_pubkey();
    
    // Hash the script with SHA256
    let mut engine = Sha256::engine();
    engine.input(&script.as_bytes());  // Convert Script to bytes
    let hash = Sha256::from_engine(engine);
    
    // Get the bytes and reverse them
    let mut bytes = hash.to_byte_array();
    bytes.reverse();
    
    // Convert to hex string
    let hex_string = hex::encode(bytes);
    
    Ok(hex_string)
}

// Function to get balance for a list of addresses at a specific date
pub fn get_balance_at_date(client: &Client, addresses: &[String], target_date: &str) -> Result<HashMap<String, f64>, String> {
    println!("Starting get_balance_at_date for {} addresses at {}", addresses.len(), target_date);
    
    // Parse the target date
    let target_datetime = match DateTime::parse_from_rfc3339(target_date) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return Err("Invalid date format. Please use RFC3339 format (e.g., 2023-01-01T00:00:00Z)".to_string()),
    };
    
    println!("Getting block height for date: {}", target_date);
    // Get the block height at the target date
    let block_height = get_block_height_at_date(client, target_datetime)?;
    println!("Block height at {} is {}", target_date, block_height);
    
    // Initialize result map
    let mut balances = HashMap::new();
    
    // Process each address
    for address_str in addresses {
        println!("Getting history for address: {}", address_str);
        
        // Convert address to scripthash
        let scripthash = address_to_scripthash(address_str)?;
        println!("Using scripthash: {}", scripthash);
        
        // Get transaction history for the address
        let bytes = match hex::decode(&scripthash) {
            Ok(b) => b,
            Err(e) => return Err(format!("Failed to decode scripthash: {}", e)),
        };
        let script = electrum_client::bitcoin::Script::from_bytes(&bytes);
        
        let history = match client.script_get_history(&script) {
            Ok(history) => history,
            Err(e) => return Err(format!("Failed to get address history: {}", e)),
        };
        
        // Calculate balance up to the target block height
        let mut balance = 0.0;
        for tx in history {
            let tx_height = tx.height as i64;
            
            // Only consider transactions confirmed before or at the target block height
            if tx_height > 0 && tx_height <= block_height {
                // Get transaction details to calculate balance change
                let tx_id_str = tx.tx_hash.to_string();
                println!("Getting details for transaction: {}", tx_id_str);
                
                let tx_id = match Txid::from_str(&tx_id_str) {
                    Ok(id) => id,
                    Err(e) => return Err(format!("Failed to parse transaction ID: {}", e)),
                };
                
                let tx_details = match client.transaction_get(&tx_id) {
                    Ok(details) => details,
                    Err(e) => return Err(format!("Failed to get transaction details: {}", e)),
                };
                
                // Calculate how this transaction affected the address balance
                let balance_change = calculate_balance_change(&tx_details, address_str);
                balance += balance_change;
            }
        }
        
        // Store the balance for this address
        balances.insert(address_str.clone(), balance / 100_000_000.0); // Convert satoshis to BTC
    }
    
    Ok(balances)
}

// Function to get block height at a specific date
fn get_block_height_at_date(client: &Client, target_date: DateTime<Utc>) -> Result<i64, String> {
    println!("Starting get_block_height_at_date for {}", target_date);
    
    // Get the current block height
    let header_info = match client.block_headers_subscribe() {
        Ok(info) => info,
        Err(e) => return Err(format!("Failed to subscribe to block headers: {}", e)),
    };
    
    let current_height = header_info.height as i64;
    println!("Current block height: {}", current_height);
    
    // Binary search to find the block at the target date
    let mut low = 0;
    let mut high = current_height;
    
    while low <= high {
        let mid = (low + high) / 2;
        println!("Checking block at height: {}", mid);
        
        // Get header for this block
        let header = match client.block_header(mid as usize) {
            Ok(header) => header,
            Err(e) => return Err(format!("Failed to get block header: {}", e)),
        };
        
        // Extract timestamp from block header
        let timestamp = header.time as i64;
        
        // Convert to DateTime
        let block_date = DateTime::<Utc>::from_timestamp(timestamp, 0)
            .unwrap_or(Utc::now());
        
        println!("Block at height {} has timestamp {}", mid, block_date);
        
        if block_date < target_date {
            low = mid + 1;
        } else if block_date > target_date {
            high = mid - 1;
        } else {
            return Ok(mid);
        }
    }
    
    // Return the closest block height
    Ok(high)
}

// Helper function to calculate how a transaction affected an address's balance
pub fn calculate_balance_change(tx: &Transaction, _address: &str) -> f64 {
    // This is a simplified implementation
    // In a real implementation, you would need to analyze inputs and outputs
    // to determine how the transaction affected the address's balance
    
    // For now, just return a placeholder value
    println!("Calculating balance change for transaction: {}", tx.compute_txid());
    0.0
}

// Function to check balance at a specific time
fn check_balance_at_time(address: &str, timestamp: u64) -> Result<f64, String> {
    let target_datetime = DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
        .unwrap_or(Utc::now());
    
    println!("\n=== Starting Balance Check ===");
    println!("Address: {}", address);
    println!("Target timestamp: {} ({})", timestamp, target_datetime.format("%Y-%m-%d %H:%M:%S UTC"));
    
    // Connect to an Electrum server
    let client = Client::new("umbrel.local:50002")
        .map_err(|e| format!("Failed to connect to Electrum server: {}", e))?;
    
    // Create a Script from the address to use with script_get_history
    let address_obj = Address::from_str(address)
        .map_err(|e| format!("Invalid address: {}", e))?
        .require_network(electrum_client::bitcoin::Network::Bitcoin)
        .map_err(|e| format!("Network error: {}", e))?;
    
    let script = address_obj.script_pubkey();
    
    println!("\n=== Getting Transaction History ===");
    // Get transaction history for the address
    let history = client.script_get_history(&script)
        .map_err(|e| format!("Failed to get history: {}", e))?;
    
    println!("Found {} total transactions", history.len());
    
    // Sort transactions by height before processing
    let mut sorted_history = history.clone();
    sorted_history.sort_by_key(|tx| tx.height);
    
    let mut balance = 0.0;
    let mut received_txs = Vec::new();
    let mut spent_txs = Vec::new();
    
    println!("\n=== Analyzing Transactions ===");
    // First pass: collect all transactions
    for (i, tx_info) in sorted_history.iter().enumerate() {
        let height = tx_info.height;
        println!("\n--- Transaction {}/{} ---", i+1, history.len());
        println!("Transaction hash: {}", tx_info.tx_hash);
        println!("Transaction height: {}", height);
        
        // Skip unconfirmed transactions or those after our target timestamp
        if height <= 0 {
            println!("Skipping unconfirmed transaction");
            continue;
        }
        
        // Get block timestamp for this transaction
        let header = match client.block_header_raw(height as usize) {
            Ok(header) => header,
            Err(e) => {
                println!("Warning: Failed to get block header: {}", e);
                continue;
            }
        };
        
        let block_timestamp = match electrum_client::bitcoin::blockdata::block::Header::consensus_decode(&mut &header[..]) {
            Ok(header) => header.time as u64,
            Err(e) => {
                println!("Warning: Failed to parse block header: {}", e);
                continue;
            }
        };
        
        let block_datetime = DateTime::<Utc>::from_timestamp(block_timestamp as i64, 0)
            .unwrap_or(Utc::now());
        println!("Block timestamp: {} ({})", block_timestamp, block_datetime.format("%Y-%m-%d %H:%M:%S UTC"));
        
        // Convert target timestamp to end of day (23:59:59)
        let target_end_of_day = timestamp + 24 * 60 * 60 - 1;
        
        // Skip transactions after the end of our target day
        if block_timestamp > target_end_of_day {
            println!("Skipping transaction - after end of target day");
            println!("Block timestamp: {} > End of day timestamp: {}", block_timestamp, target_end_of_day);
            continue;
        }
        
        println!("Processing transaction within target date range");
        
        // Get the raw transaction
        let raw_tx = client.transaction_get_raw(&tx_info.tx_hash)
            .map_err(|e| format!("Failed to get raw transaction: {}", e))?;
        
        // Parse the transaction
        let parsed_tx = electrum_client::bitcoin::Transaction::consensus_decode(&mut &raw_tx[..])
            .map_err(|e| format!("Failed to parse transaction: {}", e))?;
        
        println!("\nTransaction details:");
        println!("Number of inputs: {}", parsed_tx.input.len());
        println!("Number of outputs: {}", parsed_tx.output.len());
        
        // Check outputs (money received)
        let mut received_amount = 0.0;
        for (output_index, output) in parsed_tx.output.iter().enumerate() {
            if output.script_pubkey == script {
                let value_in_btc = output.value.to_sat() as f64 / 100_000_000.0;
                received_amount += value_in_btc;
                println!("Found receive: {} BTC (output index: {})", value_in_btc, output_index);
            }
        }
        
        // Check inputs (money spent)
        let mut spent_amount = 0.0;
        for (input_index, input) in parsed_tx.input.iter().enumerate() {
            let prev_tx_hash = input.previous_output.txid;
            println!("\nChecking input {}: Previous transaction: {}", input_index, prev_tx_hash);
            
            let prev_raw_tx = match client.transaction_get_raw(&prev_tx_hash) {
                Ok(tx) => tx,
                Err(e) => {
                    println!("Warning: Failed to get previous transaction: {}", e);
                    continue;
                }
            };
            
            let prev_parsed_tx = match electrum_client::bitcoin::Transaction::consensus_decode(&mut &prev_raw_tx[..]) {
                Ok(tx) => tx,
                Err(e) => {
                    println!("Warning: Failed to parse previous transaction: {}", e);
                    continue;
                }
            };
            
            let prev_output_index = input.previous_output.vout as usize;
            if prev_output_index < prev_parsed_tx.output.len() {
                let prev_output = &prev_parsed_tx.output[prev_output_index];
                if prev_output.script_pubkey == script {
                    let value_in_btc = prev_output.value.to_sat() as f64 / 100_000_000.0;
                    spent_amount += value_in_btc;
                    println!("Found spend: {} BTC (input index: {})", value_in_btc, input_index);
                }
            }
        }
        
        // Store transaction info if it involves our address
        if received_amount > 0.0 {
            received_txs.push((height, tx_info.tx_hash, received_amount));
            println!("\nAdded to receives: {} BTC from tx {} at height {}", received_amount, tx_info.tx_hash, height);
        }
        if spent_amount > 0.0 {
            spent_txs.push((height, tx_info.tx_hash, spent_amount));
            println!("\nAdded to spends: {} BTC from tx {} at height {}", spent_amount, tx_info.tx_hash, height);
        }
    }
    
    println!("\n=== Processing Transactions in Order ===");
    println!("Total receives to process: {}", received_txs.len());
    println!("Total spends to process: {}", spent_txs.len());
    
    // Sort receives and spends by height
    received_txs.sort_by_key(|tx| tx.0);
    spent_txs.sort_by_key(|tx| tx.0);
    
    // Process all receives first
    println!("\n--- Processing Receives ---");
    for (height, tx_hash, amount) in received_txs.iter() {
        println!("Processing receive: {} BTC from tx {} at height {}", amount, tx_hash, height);
        balance += amount;
        println!("Current balance: {} BTC", balance);
    }
    
    // Then process all spends
    println!("\n--- Processing Spends ---");
    for (height, tx_hash, amount) in spent_txs.iter() {
        println!("Processing spend: {} BTC from tx {} at height {}", amount, tx_hash, height);
        if amount > &balance {
            println!("ERROR: Attempting to spend {} BTC but only have {} BTC", amount, balance);
            println!("Transaction: {} at height {}", tx_hash, height);
            return Err("Invalid transaction sequence detected".to_string());
        }
        balance -= amount;
        println!("Current balance: {} BTC", balance);
    }
    
    println!("\n=== Final Results ===");
    println!("Final balance: {} BTC", balance);
    println!("Total receives processed: {}", received_txs.len());
    println!("Total spends processed: {}", spent_txs.len());
    Ok(balance)
}

// Helper function to find block height at a specific timestamp
pub fn find_block_height_at_timestamp(client: &Client, timestamp: u64) -> Result<i32, String> {
    // Convert timestamp to start of the day (00:00:00)
    let datetime = DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
        .unwrap_or(Utc::now());
    let start_of_day = datetime.date_naive().and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let target_timestamp = start_of_day.timestamp() as u64;

    println!("\nFinding block height for date: {} (start of day)", 
        start_of_day.format("%Y-%m-%d %H:%M:%S UTC"));
    
    // Get the current block height
    let header_info = client.block_headers_subscribe()
        .map_err(|e| format!("Failed to get current block height: {}", e))?;
    
    let current_height = header_info.height as i32;
    println!("Current block height: {}", current_height);
    
    let mut low: i32 = 0;
    let mut high: i32 = current_height;
    let mut first_valid_height: Option<i32> = None;
    
    while low <= high {
        let mid: i32 = (low + high) / 2;
        
        // Get header for this block
        let header = client.block_header_raw(mid as usize)
            .map_err(|e| format!("Failed to get block header: {}", e))?;
        
        // Extract timestamp from block header
        let block_timestamp = electrum_client::bitcoin::blockdata::block::Header::consensus_decode(&mut &header[..])
            .map_err(|e| format!("Failed to parse block header: {}", e))?
            .time as u64;
        
        let block_datetime = DateTime::<Utc>::from_timestamp(block_timestamp as i64, 0)
            .unwrap_or(Utc::now());
            
        println!("Checking height {} - block timestamp: {}", 
            mid, 
            block_datetime.format("%Y-%m-%d %H:%M:%S UTC")
        );
        
        if block_timestamp < target_timestamp {
            println!("Block too early, moving up");
            low = mid + 1;
        } else {
            // Store this height as a candidate and keep searching for earlier blocks
            first_valid_height = Some(mid);
            println!("Found valid block, but checking for earlier ones");
            high = mid - 1;
        }
    }
    
    // Return the first block that occurred on or after the start of the target day
    match first_valid_height {
        Some(height) => {
            println!("Found first block of the day at height: {}", height);
            Ok(height)
        },
        None => {
            println!("No blocks found for the target date, using last block before date");
            Ok(high)
        }
    }
}

// Helper function to convert a date string to Unix timestamp
fn date_to_timestamp(date_str: &str) -> Result<u64, String> {
    // Parse date string in format "YYYY-MM-DD HH:MM:SS"
    let naive_datetime = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| format!("Failed to parse date: {}", e))?;
    
    // Convert to UTC DateTime
    let datetime_utc = DateTime::<Utc>::from_naive_utc_and_offset(naive_datetime, Utc);
    
    // Convert to Unix timestamp
    Ok(datetime_utc.timestamp() as u64)
}

fn main() -> Result<(), String> {
    // Example usage
    //let address = "1BBZggkhPgb9Jy4f1fq2itQUKVYq9mMufY"; // address
    //let address = "1AVguosuHHSwoHC3m33w9XYqTyLA7gHJ1X";
    //let address = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
    let address = "bc1qv0gtzvsjh5tpmeehyp90dn6k8w6ddlfdtqej9c";
     


    let date_str = "2021-06-25  00:00:00";
    
    // Start measuring time
    let start_time = SystemTime::now();
    
    let timestamp = date_to_timestamp(date_str)?;
    let balance = check_balance_at_time(address, timestamp)?;
    
    // Calculate elapsed time
    let elapsed_time = start_time.elapsed()
        .map_err(|e| format!("Failed to measure time: {}", e))?;
    
    println!("Balance of {} at {} was: {} BTC", address, date_str, balance);
    println!("Time taken: {:.2} seconds", elapsed_time.as_secs_f64());
    
    Ok(())
}
