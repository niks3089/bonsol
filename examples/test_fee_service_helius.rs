// Note: This example is designed to run from the node crate workspace
// cargo run --example test_fee_service_helius

use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio;

// When running as part of the node crate, we need to import from the local modules
use bonsol_node::fee_service::{FeeEstimate, FeeService, FeeServiceConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Bonsol Fee Service with Helius RPC");
    println!("=====================================\n");

    // Get Helius RPC URL from environment
    let helius_api_key = std::env::var("HELIUS_API_KEY")
        .unwrap_or_else(|_| "ed2c2720-f40d-44d0-83be-ee7f3b8d5359".to_string());

    let helius_rpc_url = format!("https://mainnet.helius-rpc.com/?api-key={}", helius_api_key);
    println!("🌐 Using Helius RPC: {}", helius_rpc_url);

    // Create RPC client with Helius endpoint
    let rpc_client = Arc::new(RpcClient::new(helius_rpc_url));
    let config = FeeServiceConfig::default();
    let fee_service = FeeService::new(rpc_client, config);

    println!("✅ Fee service initialized with Helius RPC");
    println!(
        "   - Cache duration: {} seconds",
        config.cache_duration_secs
    );
    println!(
        "   - Default fee: {} micro-lamports/CU",
        config.default_micro_lamports_per_cu
    );
    println!(
        "   - Fallback fee: {} micro-lamports/CU\n",
        config.fallback_micro_lamports_per_cu
    );

    // Test 1: General fee estimation
    println!("📊 Test 1: General Network Fee Estimation");
    println!("------------------------------------------");

    match fee_service.estimate_general_fee().await {
        Ok(fee_estimate) => {
            println!(
                "✅ General fee estimate: {} micro-lamports per CU",
                fee_estimate.micro_lamports_per_cu
            );

            // Show cost for different transaction sizes
            let small_tx_cost = fee_service.calculate_transaction_cost(200_000, &fee_estimate);
            let medium_tx_cost = fee_service.calculate_transaction_cost(400_000, &fee_estimate);
            let large_tx_cost = fee_service.calculate_transaction_cost(800_000, &fee_estimate);

            println!(
                "   - Small transaction (200K CU): {} lamports",
                small_tx_cost
            );
            println!(
                "   - Medium transaction (400K CU): {} lamports",
                medium_tx_cost
            );
            println!(
                "   - Large transaction (800K CU): {} lamports",
                large_tx_cost
            );
        }
        Err(e) => {
            println!("❌ Failed to get general fee estimate: {}", e);
        }
    }

    println!();

    // Test 2: Account-specific fee estimation
    println!("📊 Test 2: Account-Specific Fee Estimation");
    println!("-------------------------------------------");

    // Example accounts (system program, token program, etc.)
    let test_accounts = vec![
        Pubkey::new_from_array([0; 32]), // System program
        Pubkey::new_from_array([1; 32]), // Example account
    ];

    match fee_service.estimate_proving_fee(&test_accounts).await {
        Ok(fee_estimate) => {
            println!(
                "✅ Account-specific fee estimate: {} micro-lamports per CU",
                fee_estimate.micro_lamports_per_cu
            );

            let proving_cost = fee_service.calculate_transaction_cost(400_000, &fee_estimate);
            println!("   - Proving transaction cost: {} lamports", proving_cost);
        }
        Err(e) => {
            println!("❌ Failed to get account-specific fee estimate: {}", e);
        }
    }

    println!();

    // Test 3: Profitability analysis
    println!("📊 Test 3: Profitability Analysis");
    println!("----------------------------------");

    let tip_scenarios: Vec<(&str, u64)> = vec![
        ("Low tip", 10_000),
        ("Medium tip", 50_000),
        ("High tip", 100_000),
    ];

    for (scenario, tip) in tip_scenarios {
        let general_fee = fee_service
            .estimate_general_fee()
            .await
            .unwrap_or_else(|_| FeeEstimate {
                micro_lamports_per_cu: config.fallback_micro_lamports_per_cu,
            });

        let claim_cost = fee_service.calculate_transaction_cost(200_000, &general_fee);
        let proving_cost = fee_service.calculate_transaction_cost(400_000, &general_fee);
        let is_profitable = fee_service.is_profitable(tip, claim_cost, proving_cost);

        println!("   {} ({} lamports):", scenario, tip);
        println!("     - Claim cost: {} lamports", claim_cost);
        println!("     - Proving cost: {} lamports", proving_cost);
        println!("     - Total cost: {} lamports", claim_cost + proving_cost);
        println!(
            "     - Profit: {} lamports",
            tip.saturating_sub(claim_cost + proving_cost)
        );
        println!(
            "     - Profitable: {}",
            if is_profitable { "✅ YES" } else { "❌ NO" }
        );
        println!();
    }

    println!("🏁 Test completed!");
    println!("\n💡 Usage Tips:");
    println!("   - Set HELIUS_API_KEY environment variable for your API key");
    println!("   - Run with: HELIUS_API_KEY=your_key cargo run --example test_fee_service_helius");
    println!(
        "   - Fee estimates are cached for {} seconds",
        config.cache_duration_secs
    );
    println!("   - Higher tips increase chance of winning claim competitions");

    Ok(())
}
