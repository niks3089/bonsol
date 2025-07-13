use {
    anyhow::Result,
    serde::{Deserialize, Serialize},
    solana_rpc_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::pubkey::Pubkey,
    std::{sync::Arc, time::Duration},
    tracing::{error, info, warn},
};

/// Fee information for a specific set of accounts
/// Uses 75th percentile for competitive fee estimation
#[derive(Debug, Clone)]
pub struct FeeEstimate {
    pub micro_lamports_per_cu: u64,
}

/// Configuration for fee estimation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeServiceConfig {
    pub cache_duration_secs: u64,
    pub default_micro_lamports_per_cu: u64,
    pub fallback_micro_lamports_per_cu: u64,
}

impl Default for FeeServiceConfig {
    fn default() -> Self {
        Self {
            cache_duration_secs: 5,                // Cache for 5 seconds
            default_micro_lamports_per_cu: 1_000,  // 1 micro-lamport per CU
            fallback_micro_lamports_per_cu: 5_000, // 5 micro-lamports per CU fallback
        }
    }
}

/// Service for estimating Solana transaction fees
pub struct FeeService {
    rpc_client: Arc<RpcClient>,
    config: FeeServiceConfig,
    last_general_fee_update: std::sync::Mutex<std::time::Instant>,
    cached_general_fee: std::sync::Mutex<Option<FeeEstimate>>,
}

impl FeeService {
    pub fn new(rpc_client: Arc<RpcClient>, config: FeeServiceConfig) -> Self {
        Self {
            rpc_client,
            config,
            last_general_fee_update: std::sync::Mutex::new(
                std::time::Instant::now() - Duration::from_secs(3600), // 1 hour ago
            ),
            cached_general_fee: std::sync::Mutex::new(None),
        }
    }

    /// Get fee estimate for claiming transactions
    /// This estimates the cost of sending a claim transaction
    pub async fn estimate_claim_fee(&self, execution_account: &Pubkey) -> Result<FeeEstimate> {
        // For claiming, we need to check fees for the execution account
        self.get_prioritization_fees_for_accounts(&[*execution_account])
            .await
    }

    /// Get fee estimate for proving transactions
    /// This estimates the cost of sending a proof submission transaction
    pub async fn estimate_proving_fee(&self, accounts: &[Pubkey]) -> Result<FeeEstimate> {
        // For proving, we need to check fees for all accounts involved in the transaction
        self.get_prioritization_fees_for_accounts(accounts).await
    }

    /// Get general fee estimate when no specific accounts are known
    pub async fn estimate_general_fee(&self) -> Result<FeeEstimate> {
        // Check if we have a recent cached general fee
        {
            let last_update = self.last_general_fee_update.lock().unwrap();
            if last_update.elapsed().as_secs() < self.config.cache_duration_secs {
                if let Some(cached_fee) = self.cached_general_fee.lock().unwrap().as_ref() {
                    return Ok(cached_fee.clone());
                }
            }
        }

        // Fetch new general fee information
        let fee_estimate = self.get_recent_prioritization_fees().await?;

        // Update cache
        {
            let mut last_update = self.last_general_fee_update.lock().unwrap();
            let mut cached_fee = self.cached_general_fee.lock().unwrap();
            *last_update = std::time::Instant::now();
            *cached_fee = Some(fee_estimate.clone());
        }

        Ok(fee_estimate)
    }

    /// Calculate the total cost of a transaction given compute units and fee estimate
    pub fn calculate_transaction_cost(
        &self,
        compute_units: u64,
        fee_estimate: &FeeEstimate,
    ) -> u64 {
        // Convert micro-lamports to lamports (1 lamport = 1_000_000 micro-lamports)
        (fee_estimate.micro_lamports_per_cu * compute_units) / 1_000_000
    }

    /// Check if a transaction is profitable given costs and tip
    pub fn is_profitable(&self, tip_lamports: u64, claim_cost: u64, proving_cost: u64) -> bool {
        let total_cost = claim_cost + proving_cost;
        tip_lamports > total_cost
    }

    /// Get prioritization fees for specific accounts
    async fn get_prioritization_fees_for_accounts(
        &self,
        accounts: &[Pubkey],
    ) -> Result<FeeEstimate> {
        match self
            .rpc_client
            .get_recent_prioritization_fees(accounts)
            .await
        {
            Ok(fees) => {
                if fees.is_empty() {
                    warn!("No prioritization fees returned for accounts, using default");
                    return Ok(FeeEstimate {
                        micro_lamports_per_cu: self.config.default_micro_lamports_per_cu,
                    });
                }

                // Calculate 75th percentile from the recent fees
                let mut fee_values: Vec<u64> = fees.iter().map(|f| f.prioritization_fee).collect();
                fee_values.sort_unstable();

                let len = fee_values.len();
                let percentile_75 = if len > 0 {
                    fee_values[len * 75 / 100]
                } else {
                    self.config.default_micro_lamports_per_cu
                };

                Ok(FeeEstimate {
                    micro_lamports_per_cu: percentile_75,
                })
            }
            Err(e) => {
                error!("Failed to get prioritization fees for accounts: {:?}", e);
                // Return fallback fee estimate
                Ok(FeeEstimate {
                    micro_lamports_per_cu: self.config.fallback_micro_lamports_per_cu,
                })
            }
        }
    }

    /// Get recent prioritization fees (general)
    async fn get_recent_prioritization_fees(&self) -> Result<FeeEstimate> {
        match self.rpc_client.get_recent_prioritization_fees(&[]).await {
            Ok(fees) => {
                if fees.is_empty() {
                    warn!("No recent prioritization fees available, using default");
                    return Ok(FeeEstimate {
                        micro_lamports_per_cu: self.config.default_micro_lamports_per_cu,
                    });
                }

                // Calculate 75th percentile from the recent fees
                let mut fee_values: Vec<u64> = fees.iter().map(|f| f.prioritization_fee).collect();
                fee_values.sort_unstable();

                let len = fee_values.len();
                let percentile_75 = if len > 0 {
                    fee_values[len * 75 / 100]
                } else {
                    self.config.default_micro_lamports_per_cu
                };

                info!(
                    "Updated general fee estimate: {} micro-lamports per CU (75th percentile)",
                    percentile_75
                );

                Ok(FeeEstimate {
                    micro_lamports_per_cu: percentile_75,
                })
            }
            Err(e) => {
                error!("Failed to get recent prioritization fees: {:?}", e);
                // Return fallback fee estimate
                Ok(FeeEstimate {
                    micro_lamports_per_cu: self.config.fallback_micro_lamports_per_cu,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_transaction_cost() {
        let fee_service = FeeService::new(
            Arc::new(RpcClient::new("http://localhost:8899".to_string())),
            FeeServiceConfig::default(),
        );

        let fee_estimate = FeeEstimate {
            micro_lamports_per_cu: 1_500, // 75th percentile
        };

        // Test with 200,000 compute units (typical transaction)
        let cost = fee_service.calculate_transaction_cost(200_000, &fee_estimate);
        // Should be (1_500 * 200_000) / 1_000_000 = 300 lamports
        assert_eq!(cost, 300);
    }

    #[test]
    fn test_is_profitable() {
        let fee_service = FeeService::new(
            Arc::new(RpcClient::new("http://localhost:8899".to_string())),
            FeeServiceConfig::default(),
        );

        // Case where it's profitable
        assert!(fee_service.is_profitable(1000, 300, 200)); // tip=1000, costs=500 total

        // Case where it's not profitable
        assert!(!fee_service.is_profitable(400, 300, 200)); // tip=400, costs=500 total

        // Edge case where costs equal tip
        assert!(!fee_service.is_profitable(500, 300, 200)); // tip=500, costs=500 total
    }

    #[tokio::test]
    async fn test_fee_service_with_mock_data() {
        let fee_service = FeeService::new(
            Arc::new(RpcClient::new("http://localhost:8899".to_string())),
            FeeServiceConfig::default(),
        );

        // Test fallback behavior when RPC calls fail
        let fake_account = Pubkey::new_unique();
        let result = fee_service.estimate_claim_fee(&fake_account).await;

        // Should fallback to default configuration
        assert!(result.is_ok());
        let fee_estimate = result.unwrap();
        assert_eq!(fee_estimate.micro_lamports_per_cu, 5_000); // fallback value
    }
}

/*
===== LOCAL TESTING GUIDE =====

## Prerequisites

1. Install Solana CLI:
   ```bash
   sh -c "$(curl -sSfL https://release.solana.com/v1.18.0/install)"
   ```

2. Start local test validator:
   ```bash
   solana-test-validator --reset
   ```

## Method 1: Unit Testing

Run the built-in tests:
```bash
cargo test fee_service --package bonsol-node
```

## Method 2: Integration Testing Script

Create a test script `test_fee_service.rs`:

```rust
use bonsol_node::fee_service::{FeeService, FeeServiceConfig};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to local test validator
    let rpc_client = Arc::new(RpcClient::new("http://localhost:8899".to_string()));
    let config = FeeServiceConfig::default();
    let fee_service = FeeService::new(rpc_client, config);

    // Test 1: General fee estimation
    println!("Testing general fee estimation...");
    match fee_service.estimate_general_fee().await {
        Ok(fee) => println!("✅ General fee: {} micro-lamports/CU", fee.micro_lamports_per_cu),
        Err(e) => println!("❌ General fee error: {}", e),
    }

    // Test 2: Account-specific fee estimation
    println!("\nTesting account-specific fee estimation...");
    let test_account = Pubkey::new_unique();
    match fee_service.estimate_claim_fee(&test_account).await {
        Ok(fee) => println!("✅ Claim fee: {} micro-lamports/CU", fee.micro_lamports_per_cu),
        Err(e) => println!("❌ Claim fee error: {}", e),
    }

    // Test 3: Multiple accounts (proving fee)
    println!("\nTesting proving fee estimation...");
    let test_accounts = vec![Pubkey::new_unique(), Pubkey::new_unique()];
    match fee_service.estimate_proving_fee(&test_accounts).await {
        Ok(fee) => println!("✅ Proving fee: {} micro-lamports/CU", fee.micro_lamports_per_cu),
        Err(e) => println!("❌ Proving fee error: {}", e),
    }

    // Test 4: Cost calculations
    println!("\nTesting cost calculations...");
    let fee_estimate = fee_service.estimate_general_fee().await?;

    let claim_cost = fee_service.calculate_transaction_cost(200_000, &fee_estimate);
    let proving_cost = fee_service.calculate_transaction_cost(400_000, &fee_estimate);

    println!("Claim cost (200k CU): {} lamports", claim_cost);
    println!("Proving cost (400k CU): {} lamports", proving_cost);

    // Test 5: Profitability check
    println!("\nTesting profitability...");
    let tip = 10_000; // 10k lamports tip
    let is_profitable = fee_service.is_profitable(tip, claim_cost, proving_cost);
    println!("Tip: {} lamports, Total cost: {} lamports, Profitable: {}",
             tip, claim_cost + proving_cost, is_profitable);

    Ok(())
}
```

Run the test:
```bash
cargo run --bin test_fee_service
```

## Method 3: Testing with Live Network

Test against Solana devnet:
```rust
let rpc_client = Arc::new(RpcClient::new("https://api.devnet.solana.com".to_string()));
```

## Method 4: Testing Fee Caching

Test the 5-second cache:
```rust
let start = std::time::Instant::now();
let fee1 = fee_service.estimate_general_fee().await?;
let fee2 = fee_service.estimate_general_fee().await?;
println!("Cache test - Duration: {:?}ms", start.elapsed().as_millis());
println!("Same fee returned: {}", fee1.micro_lamports_per_cu == fee2.micro_lamports_per_cu);
```

## Method 5: Testing with Real Bonsol Node

Add to your node configuration:
```rust
let fee_service = Arc::new(FeeService::new(
    rpc_client.clone(),
    config.fee_service_config.clone(),
));

// Test in execution request handler
let execution_account = accounts[2];
let claim_fee = fee_service.estimate_claim_fee(&execution_account).await?;
let proving_fee = fee_service.estimate_proving_fee(&callback_accounts).await?;

println!("Claim fee: {} micro-lamports/CU", claim_fee.micro_lamports_per_cu);
println!("Proving fee: {} micro-lamports/CU", proving_fee.micro_lamports_per_cu);
```

## Expected Behavior

### Local Test Validator
- Should return fallback fees (5,000 micro-lamports/CU) since no real priority fees exist
- All methods should complete without errors

### Live Network (Devnet/Mainnet)
- Should return real 75th percentile fees
- Values typically range from 1,000 to 100,000+ micro-lamports/CU
- Higher fees during network congestion

### Performance Expectations
- First call: ~100-500ms (RPC call)
- Cached calls: ~1-5ms (cache hit)
- Cache expires after 5 seconds
*/
