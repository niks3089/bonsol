use {
    anyhow::Result,
    serde::{Deserialize, Serialize},
    solana_rpc_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::pubkey::Pubkey,
    std::{
        sync::{Arc, Mutex},
        time::Duration,
    },
    tracing::{error, info, warn},
};
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
    pub percentile: u8,
}

impl Default for FeeServiceConfig {
    fn default() -> Self {
        Self {
            cache_duration_secs: 5,                // Cache for 5 seconds
            default_micro_lamports_per_cu: 1_000,  // 1 micro-lamport per CU
            fallback_micro_lamports_per_cu: 5_000, // 5 micro-lamports per CU fallback
            percentile: 75,                        // Use 75th percentile by default
        }
    }
}

pub struct FeeService {
    rpc_client: Arc<RpcClient>,
    config: FeeServiceConfig,
    last_general_fee_update: Mutex<std::time::Instant>,
    cached_general_fee: Mutex<Option<FeeEstimate>>,
}

impl FeeService {
    pub fn new(rpc_client: Arc<RpcClient>, config: FeeServiceConfig) -> Self {
        info!(
            "Initializing FeeService with config: default_cu={}, fallback_cu={}, percentile={}%",
            config.default_micro_lamports_per_cu,
            config.fallback_micro_lamports_per_cu,
            config.percentile
        );
        Self {
            rpc_client,
            config,
            last_general_fee_update: Mutex::new(
                std::time::Instant::now() - Duration::from_secs(3600), // 1 hour ago
            ),
            cached_general_fee: Mutex::new(None),
        }
    }

    /// Get fee estimate for claiming transactions
    /// This estimates the cost of sending a claim transaction
    pub async fn estimate_claim_fee(&self, execution_account: &Pubkey) -> Result<FeeEstimate> {
        info!(
            "Estimating claim fee for execution account: {}",
            execution_account
        );
        // For claiming, we need to check fees for the execution account
        match self
            .get_prioritization_fees_for_accounts(&[*execution_account])
            .await
        {
            Ok(estimate) => {
                info!(
                    "Successfully estimated claim fee: {} micro-lamports per CU",
                    estimate.micro_lamports_per_cu
                );
                Ok(estimate)
            }
            Err(e) => {
                error!("Error in estimate_claim_fee: {:?}", e);
                Err(e)
            }
        }
    }

    /// Get fee estimate for proving transactions
    /// This estimates the cost of sending a proof submission transaction
    pub async fn estimate_proving_fee(&self, accounts: &[Pubkey]) -> Result<FeeEstimate> {
        // For proving, we need to check fees for all accounts involved in the transaction
        self.get_prioritization_fees_for_accounts(accounts).await
    }

    /// Get general fee estimate when no specific accounts are known
    pub async fn estimate_general_fee(&self) -> Result<FeeEstimate> {
        {
            let last_update = self.last_general_fee_update.lock().unwrap();
            if last_update.elapsed().as_secs() < self.config.cache_duration_secs {
                if let Some(cached_fee) = self.cached_general_fee.lock().unwrap().as_ref() {
                    return Ok(cached_fee.clone());
                }
            }
        }

        let fee_estimate = self.get_recent_prioritization_fees().await?;

        {
            let mut last_update = self.last_general_fee_update.lock().unwrap();
            let mut cached_fee = self.cached_general_fee.lock().unwrap();
            *last_update = std::time::Instant::now();
            *cached_fee = Some(fee_estimate.clone());
        }

        Ok(fee_estimate)
    }

    pub fn calculate_transaction_cost(
        &self,
        compute_units: u64,
        fee_estimate: &FeeEstimate,
    ) -> u64 {
        // Convert micro-lamports to lamports (1 lamport = 1_000_000 micro-lamports)
        let cost = (fee_estimate.micro_lamports_per_cu * compute_units) / 1_000_000;
        info!(
            "Calculated transaction cost: {} lamports (CU={}, micro_lamports_per_cu={})",
            cost, compute_units, fee_estimate.micro_lamports_per_cu
        );
        cost
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
        info!(
            "Getting prioritization fees for {} accounts",
            accounts.len()
        );
        match self
            .rpc_client
            .get_recent_prioritization_fees(accounts)
            .await
        {
            Ok(fees) => {
                info!("RPC returned {} fee entries", fees.len());
                if fees.is_empty() {
                    warn!("No prioritization fees returned for accounts, using default");
                    return Ok(FeeEstimate {
                        micro_lamports_per_cu: self.config.default_micro_lamports_per_cu,
                    });
                }

                // Calculate configured percentile from the recent fees
                let mut fee_values: Vec<u64> = fees.iter().map(|f| f.prioritization_fee).collect();
                info!("Raw fee values: {:?}", fee_values);
                fee_values.sort_unstable();

                let len = fee_values.len();
                let percentile_fee = if len > 0 {
                    let percentile_index = (len * self.config.percentile as usize) / 100;
                    // Ensure index is within bounds
                    let index = std::cmp::min(percentile_index, len - 1);
                    fee_values[index]
                } else {
                    self.config.default_micro_lamports_per_cu
                };

                info!(
                    "Selected {}th percentile fee: {} micro-lamports per CU from {} values",
                    self.config.percentile, percentile_fee, len
                );

                // If calculated fee is 0 (common in local dev), use default fallback
                let final_fee = if percentile_fee == 0 {
                    warn!(
                        "Calculated fee is 0, using default fallback: {} micro-lamports per CU",
                        self.config.default_micro_lamports_per_cu
                    );
                    self.config.default_micro_lamports_per_cu
                } else {
                    percentile_fee
                };

                Ok(FeeEstimate {
                    micro_lamports_per_cu: final_fee,
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

                // Calculate configured percentile from the recent fees
                let mut fee_values: Vec<u64> = fees.iter().map(|f| f.prioritization_fee).collect();
                fee_values.sort_unstable();

                let len = fee_values.len();
                let percentile_fee = if len > 0 {
                    let percentile_index = (len * self.config.percentile as usize) / 100;
                    // Ensure index is within bounds
                    let index = std::cmp::min(percentile_index, len - 1);
                    fee_values[index]
                } else {
                    self.config.default_micro_lamports_per_cu
                };

                info!(
                    "Updated general fee estimate: {} micro-lamports per CU ({}th percentile)",
                    percentile_fee, self.config.percentile
                );

                Ok(FeeEstimate {
                    micro_lamports_per_cu: percentile_fee,
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
            micro_lamports_per_cu: 1_500, // example fee
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

    #[test]
    fn test_percentile_calculation() {
        // Test helper function to simulate percentile calculation
        fn calculate_percentile(values: &[u64], percentile: u8) -> u64 {
            let mut sorted_values = values.to_vec();
            sorted_values.sort_unstable();

            let len = sorted_values.len();
            if len > 0 {
                let percentile_index = (len * percentile as usize) / 100;
                let index = std::cmp::min(percentile_index, len - 1);
                sorted_values[index]
            } else {
                0
            }
        }

        // Test case 1: Even distribution
        let fees = vec![100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];

        assert_eq!(calculate_percentile(&fees, 50), 500); // 50th percentile (median)
        assert_eq!(calculate_percentile(&fees, 75), 700); // 75th percentile
        assert_eq!(calculate_percentile(&fees, 90), 900); // 90th percentile
        assert_eq!(calculate_percentile(&fees, 95), 900); // 95th percentile (same as 90th for 10 values)

        // Test case 2: Single value
        let single_fee = vec![1000];
        assert_eq!(calculate_percentile(&single_fee, 75), 1000);

        // Test case 3: Two values
        let two_fees = vec![100, 200];
        assert_eq!(calculate_percentile(&two_fees, 75), 100); // 75% of 2 = 1.5 -> index 1 (min with len-1)

        // Test case 4: Empty array
        let empty_fees: Vec<u64> = vec![];
        assert_eq!(calculate_percentile(&empty_fees, 75), 0);
    }

    #[test]
    fn test_fee_service_config_default() {
        let config = FeeServiceConfig::default();

        assert_eq!(config.cache_duration_secs, 5);
        assert_eq!(config.default_micro_lamports_per_cu, 1_000);
        assert_eq!(config.fallback_micro_lamports_per_cu, 5_000);
        assert_eq!(config.percentile, 75); // Verify default is 75th percentile
    }

    #[test]
    fn test_fee_service_config_custom_percentile() {
        let config = FeeServiceConfig {
            cache_duration_secs: 10,
            default_micro_lamports_per_cu: 2_000,
            fallback_micro_lamports_per_cu: 10_000,
            percentile: 90, // Custom 90th percentile
        };

        let fee_service = FeeService::new(
            Arc::new(RpcClient::new("http://localhost:8899".to_string())),
            config.clone(),
        );

        // Verify the config is stored correctly
        assert_eq!(fee_service.config.percentile, 90);
        assert_eq!(fee_service.config.cache_duration_secs, 10);
        assert_eq!(fee_service.config.default_micro_lamports_per_cu, 2_000);
        assert_eq!(fee_service.config.fallback_micro_lamports_per_cu, 10_000);
    }

    #[test]
    fn test_edge_cases_percentile_bounds() {
        // Test edge cases for percentile calculation
        let config_0 = FeeServiceConfig {
            percentile: 0,
            ..Default::default()
        };

        let config_100 = FeeServiceConfig {
            percentile: 100,
            ..Default::default()
        };

        let fee_service_0 = FeeService::new(
            Arc::new(RpcClient::new("http://localhost:8899".to_string())),
            config_0,
        );

        let fee_service_100 = FeeService::new(
            Arc::new(RpcClient::new("http://localhost:8899".to_string())),
            config_100,
        );

        // These should not panic and should create valid fee services
        assert_eq!(fee_service_0.config.percentile, 0);
        assert_eq!(fee_service_100.config.percentile, 100);
    }
}
