# Fee Service Examples

This directory contains examples for testing the Bonsol fee service with different RPC endpoints.

## Test Fee Service with Helius RPC

### Quick Start

```bash
# Run with default API key
cargo run --example test_fee_service_helius

# Run with custom API key
HELIUS_API_KEY=your_custom_key cargo run --example test_fee_service_helius
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `HELIUS_API_KEY` | Your Helius API key | `ed2c2720-f40d-44d0-83be-ee7f3b8d5359` |

### Helius Endpoints

The example uses the following Helius endpoints:

- **RPC**: `https://mainnet.helius-rpc.com/?api-key=YOUR_KEY`
- **Streaming**: `https://laserstream-mainnet-pitt.helius-rpc.com?api-key=YOUR_KEY`

### What the Example Tests

1. **General Fee Estimation**: Gets current network priority fees (75th percentile)
2. **Account-Specific Fees**: Estimates fees for specific accounts (higher for hot accounts)
3. **Profitability Analysis**: Shows whether different tip amounts are profitable

### Sample Output

```
🚀 Bonsol Fee Service with Helius RPC
=====================================

🌐 Using Helius RPC: https://mainnet.helius-rpc.com/?api-key=ed2c2720-f40d-44d0-83be-ee7f3b8d5359
✅ Fee service initialized with Helius RPC
   - Cache duration: 5 seconds
   - Default fee: 1000 micro-lamports/CU
   - Fallback fee: 5000 micro-lamports/CU

📊 Test 1: General Network Fee Estimation
------------------------------------------
✅ General fee estimate: 2500 micro-lamports per CU
   - Small transaction (200K CU): 500 lamports
   - Medium transaction (400K CU): 1000 lamports
   - Large transaction (800K CU): 2000 lamports

📊 Test 2: Account-Specific Fee Estimation
-------------------------------------------
✅ Account-specific fee estimate: 3200 micro-lamports per CU
   - Proving transaction cost: 1280 lamports

📊 Test 3: Profitability Analysis
----------------------------------
   Low tip (10000 lamports):
     - Claim cost: 500 lamports
     - Proving cost: 1000 lamports
     - Total cost: 1500 lamports
     - Profit: 8500 lamports
     - Profitable: ✅ YES

   Medium tip (50000 lamports):
     - Claim cost: 500 lamports
     - Proving cost: 1000 lamports
     - Total cost: 1500 lamports
     - Profit: 48500 lamports
     - Profitable: ✅ YES

   High tip (100000 lamports):
     - Claim cost: 500 lamports
     - Proving cost: 1000 lamports
     - Total cost: 1500 lamports
     - Profit: 98500 lamports
     - Profitable: ✅ YES
```

### Key Features

- **No Business Logic Changes**: Uses existing `FeeService` without modifications
- **Environment-Based Configuration**: Easy to switch between endpoints
- **Real-time Data**: Fetches live priority fee data from Helius
- **75th Percentile Strategy**: Competitive fee estimation for claim races
- **Caching**: 5-second cache to avoid excessive RPC calls
- **Profitability Analysis**: Helps determine if execution requests are worth claiming

### Testing Tips

1. **Network Conditions**: Fee estimates vary with network congestion
2. **Account Hotness**: Popular accounts have higher priority fees
3. **Timing**: Fees change rapidly - cache prevents stale data
4. **Competition**: Higher fees increase chance of winning claim races
5. **Profitability**: Always factor in computational costs (not included in transaction fees)