# Testnet Deployment Evidence: Current Contracts (4 October 2026)

Status: this record describes the deployment of the current Tranche 1 contracts to Stellar Testnet, made on 4 October 2026. It replaces the earlier eight-contract record as the reference for the current code. The earlier records remain in the repository as history: [TESTNET_DEPLOYMENT_EVIDENCE.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TESTNET_DEPLOYMENT_EVIDENCE.md) and [TESTNET_STRESS_TEST_EVIDENCE.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TESTNET_STRESS_TEST_EVIDENCE.md).

## 1. What was deployed, and from which code

- Network: Stellar Testnet, passphrase `Test SDF Network ; September 2015`, RPC `https://soroban-testnet.stellar.org`.
- Source: the eleven contracts in the reviewed commit. The contract source is identical to commit [6cf257a](https://github.com/principal-Protocol-org/principal/commit/6cf257afb588c63d694f2262d8b289d4809be8ee). `git diff 6cf257a HEAD -- contracts` is empty. Only documentation changed after the review commit.
- Build: `cargo build --workspace --release --target wasm32v1-none`, eleven WASM files, the largest 66,420 bytes (MarketPool), all under the 128 KiB limit.
- Procedure: the order and configuration in [DEPLOYMENT.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/DEPLOYMENT.md), with the corrections recorded in section 6.
- Underlying asset: the STA classic asset with `AUTH_REQUIRED` and `AUTH_REVOCABLE` set, wrapped by a Stellar Asset Contract. It stands in for the yield-bearing USDY asset named in the runbook. Its administrator is the issuer account, which signs every administrator action.
- Testnet only. The administrator, treasury and all users are Testnet keys. No mainnet account was used.

## 2. Accounts

| Role | Address | Explorer |
|---|---|---|
| Issuer, SAC administrator, protocol admin for every contract, creator | `GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB` | [account](https://stellar.expert/explorer/testnet/account/GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB) |
| Protocol admin (MarketConfig) | `GAZHOIUXZGGEW3CSTE2EDVMM6FX65EBN3JW3J5AV225WC5QCHLCHYZO6` | [account](https://stellar.expert/explorer/testnet/account/GAZHOIUXZGGEW3CSTE2EDVMM6FX65EBN3JW3J5AV225WC5QCHLCHYZO6) |
| Treasury (fee recipient) | `GASQCEPSJJFWY5ITMQGNOVXQOGIQK2CBG6KBMHT7EMHEMSP7MHMYYAGR` | [account](https://stellar.expert/explorer/testnet/account/GASQCEPSJJFWY5ITMQGNOVXQOGIQK2CBG6KBMHT7EMHEMSP7MHMYYAGR) |
| Alice (ordinary holder) | `GCXV54A7CAMBJVKQRB7IZSVWJFBLGUK556DU6CZSMS337QHHNXBP7OLT` | [account](https://stellar.expert/explorer/testnet/account/GCXV54A7CAMBJVKQRB7IZSVWJFBLGUK556DU6CZSMS337QHHNXBP7OLT) |
| Bob (later deauthorised and recovered) | `GBYSZUHZQNWJQJIPZJ7NAZGJZB2CCFP2VOXGOGNJVLROJOQ2XEM4U55N` | [account](https://stellar.expert/explorer/testnet/account/GBYSZUHZQNWJQJIPZJ7NAZGJZB2CCFP2VOXGOGNJVLROJOQ2XEM4U55N) |
| Carol (liquidity provider and trader) | `GACRHVJ2VD5GWX57CWWS373MXOWH47GSDSITYPXAUWCDKVKFYVM2YVU5` | [account](https://stellar.expert/explorer/testnet/account/GACRHVJ2VD5GWX57CWWS373MXOWH47GSDSITYPXAUWCDKVKFYVM2YVU5) |

## 3. Contract addresses

Final market, the one used for every flow in section 4:

| Contract | Address | Explorer |
|---|---|---|
| SYWrapper | `CDKKMKVBNJKNQ5O2ENX5UORUDNBPGA2QV72SWIWOCHSIJ52IBJEJJEQT` | [contract](https://stellar.expert/explorer/testnet/contract/CDKKMKVBNJKNQ5O2ENX5UORUDNBPGA2QV72SWIWOCHSIJ52IBJEJJEQT) |
| PTToken | `CCOC5N37SVJ7OVG5ZM2U4SO4BOCAXWEOI7235U6333X2H7Q3WYM3QMRD` | [contract](https://stellar.expert/explorer/testnet/contract/CCOC5N37SVJ7OVG5ZM2U4SO4BOCAXWEOI7235U6333X2H7Q3WYM3QMRD) |
| YTToken | `CAULNVSPNAQWKVV2YKIPQC2NUGPVMJ7SS2PPVKFCKCLUN2AE5QLFELHD` | [contract](https://stellar.expert/explorer/testnet/contract/CAULNVSPNAQWKVV2YKIPQC2NUGPVMJ7SS2PPVKFCKCLUN2AE5QLFELHD) |
| MarketConfig | `CBKWPYFGNISVQSDLSQCDJIOMXODZTJ22UV7J7HHZRCBYEWM6AQXMEQOI` | [contract](https://stellar.expert/explorer/testnet/contract/CBKWPYFGNISVQSDLSQCDJIOMXODZTJ22UV7J7HHZRCBYEWM6AQXMEQOI) |
| PrincipalManager | `CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27` | [contract](https://stellar.expert/explorer/testnet/contract/CDVVB6OKHQ7YV772VFZ4C3GQOHX34JPYIMC4AT353IGMUECYOWILAV27) |
| MarketPool | `CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR` | [contract](https://stellar.expert/explorer/testnet/contract/CANAIVIXZXGCQCCXX5KKEF567O5RZ3GDFFKQJOZPYSAZWIJBEIHRIIPR) |
| RecoveryEscrow | `CD23JCOPNIT26KAEVQI3EKEAUZ7RS2M5IDW7MTDZ6B7O3OAGOWU5ANBG` | [contract](https://stellar.expert/explorer/testnet/contract/CD23JCOPNIT26KAEVQI3EKEAUZ7RS2M5IDW7MTDZ6B7O3OAGOWU5ANBG) |
| Router | `CARTAP66PCU6GJJ4MGWATZM54YIT4RF7WNTPCA2KHMQD6W23NNU7KC2U` | [contract](https://stellar.expert/explorer/testnet/contract/CARTAP66PCU6GJJ4MGWATZM54YIT4RF7WNTPCA2KHMQD6W23NNU7KC2U) |

Shared with the final market (not tied to maturity):

| Contract | Address | Explorer |
|---|---|---|
| OracleAdapter | `CAW53CGHBDS3DS6JJ7NOWSTCJCKNN663FEEN3WOYWJKMDOFAALZIXCRI` | [contract](https://stellar.expert/explorer/testnet/contract/CAW53CGHBDS3DS6JJ7NOWSTCJCKNN663FEEN3WOYWJKMDOFAALZIXCRI) |
| Permissioning | `CDX64AORDY5FMDPQH633WIMNH5NBABW34UIXEDIS5E3IFWU7QFQ6YMCF` | [contract](https://stellar.expert/explorer/testnet/contract/CDX64AORDY5FMDPQH633WIMNH5NBABW34UIXEDIS5E3IFWU7QFQ6YMCF) |
| RiskControl | `CCQKY4S4TC5G3LZIT37I5IBSSDVEZIIVPZSMCJVSOTASGKC6XKTZKLYR` | [contract](https://stellar.expert/explorer/testnet/contract/CCQKY4S4TC5G3LZIT37I5IBSSDVEZIIVPZSMCJVSOTASGKC6XKTZKLYR) |
| STA Stellar Asset Contract | `CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L` | [contract](https://stellar.expert/explorer/testnet/contract/CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L) |

Superseded (kept on the ledger, not used by the final market): the first market's PT, YT, PrincipalManager, MarketPool, RecoveryEscrow, Router and SYWrapper (aliases `v11_*`), and the second market's PT, YT, MarketConfig and PrincipalManager (aliases `m2_*`). Their addresses are in the aliases recorded in the log below. The second market's SYWrapper was the first market's, which is why a second market could not be wired without a new SYWrapper (see section 6).

## 4. What the final market did, on-chain

Each step below is one transaction, and the full list is in section 7.

- Deposits and mints. Alice deposited 500 STA and minted PT and YT. The 0.05% tokenization fee left her with 4,997,500,000 raw PT and 4,997,500,000 raw YT, that is 499.75 each. Bob deposited 300 STA and minted 2,998,500,000 PT and YT. Carol deposited 400 STA and minted 200 PT and 200 YT, which is 1,999,000,000 raw each after the fee.
- Liquidity. Carol added 100 PT and 100 SY to the pool.
- Trades. Alice sold 100 PT to the pool for SY. Carol bought PT with 30 SY. Carol flash-minted YT through the Router with 20 SY, which mints PT and YT and sells the PT to the pool.
- Yield. The oracle moved from 1.00 to 1.05 (`set_reference_value` to 10,500,000). Alice claimed her yield mid-life.
- Compliance recovery. The issuer deauthorised Bob on the SAC. Then `seize_all_positions` moved his PT and YT into the escrow in one transaction.
- Maturity. The market matured at ledger time 1791056747. The oracle was refreshed, `settle_all` froze the settlement rate, and then Alice redeemed 3,997,500,000 PT and 4,997,500,000 YT.
- Exit. Carol removed all of her liquidity (999,999,000 LP). The treasury and the creator claimed their fees from the pool.
- Recovery finalised. `finalize_record` for record 0 settled Bob's position into the escrow. The record shows 2,855,714,285 raw from the PT leg, 128,507,142 raw from the YT leg, and 142,785,714 raw of yield attributed to the seizure. The escrow holds 2,984,221,427 raw units, which is the sum of the two legs.

Expired-market check. After maturity, a PT-for-SY swap by Alice was refused by the pool with contract error 6. The refusal happened at simulation, so there is no transaction hash for it, and the CLI did not submit it.

Circuit breaker. The RiskControl volume read on-chain is 27,200,000,000 raw units. This is cumulative across the shared breaker, so it includes the earlier superseded attempts as well as the final market. SYWrapper and PrincipalManager call RiskControl from inside their own entry points, since both are registered consumers, so the deposits and mints in this run were counted without any separate call. The window is 17,280 ledgers and the protocol-wide limit is 1,000,000,000,000 raw units. The record does not show a deposit being refused on-chain, so the automatic revert is not demonstrated here; it is covered by the tests named in [TRANCHE_1_DELIVERABLES.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/TRANCHE_1_DELIVERABLES.md).

## 5. Final on-chain state, read from the contracts

- PrincipalManager and PT supply: 3,198,900,000 PT. YT supply: 2,198,900,000 YT.
- The gap of 1,000,000,000 is not an error. Alice redeemed 4,997,500,000 YT but only 3,997,500,000 PT, because 100 PT had been sold to the pool. That PT is now held by Carol. Redemption accepts unequal PT and YT amounts by design, so PT and YT supplies need not match.
- Carol holds 3,198,898,100 PT and 2,198,900,000 YT. The pool holds 1,900 PT of dust.
- Alice and Bob hold no PT and no YT. The escrow holds no PT and no YT.
- The STA balance of the escrow is 2,984,221,427 raw units (298.4221427 STA). This is the amount the finalised record settled.
- Bob's SAC authorisation reads `false`. Its trustline is not authorised, so he cannot receive STA.
- The STA asset has `auth_clawback_enabled: false`. The issuer cannot claw back on this asset. The recovered STA stays in the escrow, and the native clawback step cannot be run on this asset as configured.

## 6. Findings from the deployment

These are recorded as they happened, including the mistakes. None of them changes a contract. Each one is a process or documentation gap, and the runbook is corrected accordingly.

1. Wiring is one-time and not replaceable. `SYWrapper.set_recovery_escrow` reverts `RecoveryEscrowAlreadySet` (error 10) once set, and PrincipalManager stores its SYWrapper address at initialization. The first market's SYWrapper was therefore bound to that market's escrow, so a second market could not reuse it. A replacement market needed a new SYWrapper and every dependent contract redeployed. Operators should treat SYWrapper as a per-market component and choose its escrow before the first use.
2. Maturity is fixed at initialization for PT, YT, MarketConfig and PrincipalManager. The first market's 45-minute window expired before the lifecycle could run, because the setup took longer than expected. It was then superseded. The final market was given a 20-minute window.
3. Fee payees need Permissioning standing. The runbook's step 11 requires the treasury and the creator to hold a Permissioning grant. The fee claims failed with error 8 until both were granted (transactions in section 7, rows 'permissioning.grant_account.treasury' and 'permissioning.grant_account.creator'). The runbook was not followed exactly in the first pass, and it now states the requirement explicitly.
4. Redemption accepts unequal PT and YT amounts. This is by design but surprises a reader of the supplies. Section 5 explains the 1,000,000,000 gap.
5. The redeem call failed first on an incorrect amount. The redemption was requested for 4,000,000,000 PT, but Alice's balance was 3,997,500,000 after the fee and the sale. The contract refused it with `InsufficientBalance` (PT error 5) at simulation, so nothing was submitted. Balances should be read before redeeming.
6. Testnet RPC timed out once during deployment (`Request timeout`). The affected step was retried from the start with a fresh prefix. Aliases are only written on success, so the retry did not duplicate any contract.
7. Oracle freshness. Every mint, redeem and settle requires a value younger than 3,600 seconds. Each step in this run refreshed the oracle first. A relay would be needed for a long-running market.
8. Timestamps. The oracle rejects a timestamp later than the ledger's close time (error 7, `TimestampInFuture`). The local clock was about one second ahead of the ledger, so timestamps were taken from the latest ledger close time minus five seconds.

## 7. Verification of every transaction

Every transaction hash in the table below was checked against the Horizon Testnet API on 4 October 2026. The check result is recorded in section 8.

Rows are grouped as follows. "Shared infrastructure" is used by the final market. "Final market" is the 20-minute market. "Superseded attempt" marks transactions from the first and second markets, which are kept as history.

| # | Group | Step | Transaction |
|---|---|---|---|
| 1 | Shared infrastructure | `v11 oracle_adapter upload` | [48a02be34d207b2befdc36d4ba08ecee7a285ea4a69b84307acb718bf62a698f](https://stellar.expert/explorer/testnet/tx/48a02be34d207b2befdc36d4ba08ecee7a285ea4a69b84307acb718bf62a698f) |
| 2 | Shared infrastructure | `v11 oracle_adapter deploy` | [866b1d40a6ad2ccf08a27f30ea11e026a0d8a92913c4490c90e830ea069fea5c](https://stellar.expert/explorer/testnet/tx/866b1d40a6ad2ccf08a27f30ea11e026a0d8a92913c4490c90e830ea069fea5c) |
| 3 | Shared infrastructure | `v11_oracle_adapter.initialize` | [fe50e072db057158439ee23d9e8b41b8dbbfc1ea5ce2c7d168e04e54e73356e2](https://stellar.expert/explorer/testnet/tx/fe50e072db057158439ee23d9e8b41b8dbbfc1ea5ce2c7d168e04e54e73356e2) |
| 4 | Shared infrastructure | `v11_oracle_adapter.set_reference_value.genesis` | [72f1c91197e495d0f3a3ffe4bc96da3af4379638ab680ba7a399c0fa18d6ee19](https://stellar.expert/explorer/testnet/tx/72f1c91197e495d0f3a3ffe4bc96da3af4379638ab680ba7a399c0fa18d6ee19) |
| 5 | Shared infrastructure | `v11_permissioning.deploy` | [9d7513602741f1ab9ea80fd34b3171e4011805f2c1e605e1633bcdd29112a902](https://stellar.expert/explorer/testnet/tx/9d7513602741f1ab9ea80fd34b3171e4011805f2c1e605e1633bcdd29112a902) |
| 6 | Shared infrastructure | `v11_permissioning.initialize` | [597d25f56416b442ecf40e1f012f4859db1b9b0445643a97749de157c0af2c29](https://stellar.expert/explorer/testnet/tx/597d25f56416b442ecf40e1f012f4859db1b9b0445643a97749de157c0af2c29) |
| 7 | Shared infrastructure | `v11_risk_control.deploy` | [f80b02885f8e24a5a0a72a00f2477f03614aba110b57710ad3f4770ace12d1ef](https://stellar.expert/explorer/testnet/tx/f80b02885f8e24a5a0a72a00f2477f03614aba110b57710ad3f4770ace12d1ef) |
| 8 | Shared infrastructure | `v11_risk_control.initialize` | [35a39fdcb4dc070ec49426f2ae1c8dc286cd9170f00e20fc904ca3b343411b62](https://stellar.expert/explorer/testnet/tx/35a39fdcb4dc070ec49426f2ae1c8dc286cd9170f00e20fc904ca3b343411b62) |
| 9 | Superseded attempt | `v11_sy_wrapper.deploy` | [0ce6a98b342d5f336ba826ca8e4cdb6e173cce15d5a66bea4398ab2a6555b039](https://stellar.expert/explorer/testnet/tx/0ce6a98b342d5f336ba826ca8e4cdb6e173cce15d5a66bea4398ab2a6555b039) |
| 10 | Superseded attempt | `v11_sy_wrapper.initialize` | [c9ee7ccf0922697b52054b70514a5caa74da5f9f93d6225b9e98749a7a28ffd9](https://stellar.expert/explorer/testnet/tx/c9ee7ccf0922697b52054b70514a5caa74da5f9f93d6225b9e98749a7a28ffd9) |
| 11 | Superseded attempt | `v11_sy_wrapper.set_risk_control` | [abab09553a9e820e0554b788e6d626779def49bdb4742e31a8a0cc796d969b80](https://stellar.expert/explorer/testnet/tx/abab09553a9e820e0554b788e6d626779def49bdb4742e31a8a0cc796d969b80) |
| 12 | Superseded attempt | `v11_risk_control.add_consumer.sy_wrapper` | [d82e15711c192f3f7756ca55cc8fb37c8f16eaf5a429635cba1cc48d26a89960](https://stellar.expert/explorer/testnet/tx/d82e15711c192f3f7756ca55cc8fb37c8f16eaf5a429635cba1cc48d26a89960) |
| 13 | Superseded attempt | `v11_pt_token.deploy` | [2d3f69e8983d48600fde5e8359da76a6d3cc56cd721c1a166c9942276619e501](https://stellar.expert/explorer/testnet/tx/2d3f69e8983d48600fde5e8359da76a6d3cc56cd721c1a166c9942276619e501) |
| 14 | Superseded attempt | `v11_pt_token.initialize` | [e62a63a6eddff8c0dffceb17cc0f97bc8c68866a8b2ad691b0d8a9ef89d3a22c](https://stellar.expert/explorer/testnet/tx/e62a63a6eddff8c0dffceb17cc0f97bc8c68866a8b2ad691b0d8a9ef89d3a22c) |
| 15 | Superseded attempt | `v11_yt_token.deploy` | [61a31064aad9a334bea9b1ee80f33376441ef08ee0a15f11e216b970d414e2a0](https://stellar.expert/explorer/testnet/tx/61a31064aad9a334bea9b1ee80f33376441ef08ee0a15f11e216b970d414e2a0) |
| 16 | Superseded attempt | `v11_yt_token.initialize` | [58dada196e2a209b01e03b277fa59fd68193e5d645d9baa331eb3d866d56d79b](https://stellar.expert/explorer/testnet/tx/58dada196e2a209b01e03b277fa59fd68193e5d645d9baa331eb3d866d56d79b) |
| 17 | Superseded attempt | `v11_market_config.deploy` | [7539dabd760bff5d5933cc2f586927dc86acb884831a105ecab4f4d0f494a997](https://stellar.expert/explorer/testnet/tx/7539dabd760bff5d5933cc2f586927dc86acb884831a105ecab4f4d0f494a997) |
| 18 | Superseded attempt | `v11_market_config.initialize` | [77960ba55dc0e16761c52fca36ae8c554d91c1cdd7373ec913b27d94fc1c74c0](https://stellar.expert/explorer/testnet/tx/77960ba55dc0e16761c52fca36ae8c554d91c1cdd7373ec913b27d94fc1c74c0) |
| 19 | Superseded attempt | `v11_principal_manager.deploy` | [691adfff98bdf15a51f6e9cda828d16c7818bd64edb069f2f304738dff796912](https://stellar.expert/explorer/testnet/tx/691adfff98bdf15a51f6e9cda828d16c7818bd64edb069f2f304738dff796912) |
| 20 | Superseded attempt | `v11_principal_manager.initialize` | [26b1df042226cde71a40c8f10a4baf5492931e195c9e05218e2489f9a937b5ef](https://stellar.expert/explorer/testnet/tx/26b1df042226cde71a40c8f10a4baf5492931e195c9e05218e2489f9a937b5ef) |
| 21 | Superseded attempt | `v11_pt_token.set_minter` | [0f047960c7cfc7f8d76aa4ded756bd6e405fa824033cede1d1174b1a758612b4](https://stellar.expert/explorer/testnet/tx/0f047960c7cfc7f8d76aa4ded756bd6e405fa824033cede1d1174b1a758612b4) |
| 22 | Superseded attempt | `v11_yt_token.set_minter` | [211b0ce74be76415aeea0ca7d2ca836a822ee0404fd6975ff5d7bb3905f35898](https://stellar.expert/explorer/testnet/tx/211b0ce74be76415aeea0ca7d2ca836a822ee0404fd6975ff5d7bb3905f35898) |
| 23 | Superseded attempt | `v11_principal_manager.set_risk_control` | [60c8c57976b58673fb815fca2761575b6a30de62b2614e4e81f88e47a9117fd2](https://stellar.expert/explorer/testnet/tx/60c8c57976b58673fb815fca2761575b6a30de62b2614e4e81f88e47a9117fd2) |
| 24 | Superseded attempt | `v11_risk_control.add_consumer.principal_manager` | [302ac46d33a84ba6f7271709b6965045da7a07dd70f3793400c596be0cfc48c3](https://stellar.expert/explorer/testnet/tx/302ac46d33a84ba6f7271709b6965045da7a07dd70f3793400c596be0cfc48c3) |
| 25 | Superseded attempt | `v11_market_pool.deploy` | [7f6a8a26481ad2194ea3dd31e74e1c7d44f24e41a43a504d99fe51abe6b26aeb](https://stellar.expert/explorer/testnet/tx/7f6a8a26481ad2194ea3dd31e74e1c7d44f24e41a43a504d99fe51abe6b26aeb) |
| 26 | Superseded attempt | `v11_market_pool.initialize` | [08a8742db1d5ca39598f06923499af20a196595aa30a9f9f3c6f317683414c6a](https://stellar.expert/explorer/testnet/tx/08a8742db1d5ca39598f06923499af20a196595aa30a9f9f3c6f317683414c6a) |
| 27 | Superseded attempt | `v11_recovery_escrow.deploy` | [ba96025ad97e28a2fc7ff131206b61a53e96ef76f4e6661b60ae483889f5f936](https://stellar.expert/explorer/testnet/tx/ba96025ad97e28a2fc7ff131206b61a53e96ef76f4e6661b60ae483889f5f936) |
| 28 | Superseded attempt | `v11_recovery_escrow.initialize` | [e118b327140770777f03cc5c237a84ca4cf4364abb2655be64f2e57f1ebf54fe](https://stellar.expert/explorer/testnet/tx/e118b327140770777f03cc5c237a84ca4cf4364abb2655be64f2e57f1ebf54fe) |
| 29 | Superseded attempt | `v11_sy_wrapper.set_recovery_escrow` | [7d13475729c5f41423ce9db3b5162a37f2ca5a91e452ede1a7e62d8c909f3efb](https://stellar.expert/explorer/testnet/tx/7d13475729c5f41423ce9db3b5162a37f2ca5a91e452ede1a7e62d8c909f3efb) |
| 30 | Superseded attempt | `v11_pt_token.set_recovery_escrow` | [5b9f6b03b993f6a2b1d0f482d6ecc4ad9435952e6f2cb30b43225398348e1327](https://stellar.expert/explorer/testnet/tx/5b9f6b03b993f6a2b1d0f482d6ecc4ad9435952e6f2cb30b43225398348e1327) |
| 31 | Superseded attempt | `v11_yt_token.set_recovery_escrow` | [e89645f42551898751b76d159cd71bc1c1b7d11061f1c1f72507e47f2e708ea2](https://stellar.expert/explorer/testnet/tx/e89645f42551898751b76d159cd71bc1c1b7d11061f1c1f72507e47f2e708ea2) |
| 32 | Superseded attempt | `v11_market_pool.set_recovery_escrow` | [d0c4180b5992ebdd36354dd39fd454886e9c7265ce955355686d1f5033c4b22e](https://stellar.expert/explorer/testnet/tx/d0c4180b5992ebdd36354dd39fd454886e9c7265ce955355686d1f5033c4b22e) |
| 33 | Superseded attempt | `v11_router.deploy` | [94dba106be973c581d1c7004f564e25eb87d9d5181a7c640adbf67255cfa0935](https://stellar.expert/explorer/testnet/tx/94dba106be973c581d1c7004f564e25eb87d9d5181a7c640adbf67255cfa0935) |
| 34 | Superseded attempt | `v11_router.initialize` | [14b3841497e749bb56070df7d8c1c8f4a71211d642e9424903651dd5ecf59a3e](https://stellar.expert/explorer/testnet/tx/14b3841497e749bb56070df7d8c1c8f4a71211d642e9424903651dd5ecf59a3e) |
| 35 | Superseded attempt | `v11_router.register_market` | [9bffa00d7493a4192a61108490e7a4591d1e7dce5c5679968c7eb509414fd513](https://stellar.expert/explorer/testnet/tx/9bffa00d7493a4192a61108490e7a4591d1e7dce5c5679968c7eb509414fd513) |
| 36 | Superseded attempt | `sac_authorize.v11_sy_wrapper` | [8a38eab73796b10c16f22ee41386ed3aceafdcf63a154190cdbdb14a639938d4](https://stellar.expert/explorer/testnet/tx/8a38eab73796b10c16f22ee41386ed3aceafdcf63a154190cdbdb14a639938d4) |
| 37 | Superseded attempt | `permissioning.grant_account.v11_sy_wrapper` | [df9c1d8292ee4b121b2edf5a0d2d2cfc145c46333f3eace736be1344b693db40](https://stellar.expert/explorer/testnet/tx/df9c1d8292ee4b121b2edf5a0d2d2cfc145c46333f3eace736be1344b693db40) |
| 38 | Superseded attempt | `sac_authorize.v11_principal_manager` | [8557325673b96d36f8d245a17f90c6e16f3e2f57bfdb93a1f765d5595cae4caf](https://stellar.expert/explorer/testnet/tx/8557325673b96d36f8d245a17f90c6e16f3e2f57bfdb93a1f765d5595cae4caf) |
| 39 | Superseded attempt | `permissioning.grant_account.v11_principal_manager` | [2b52fbb4253aef87526c7df9527fa7f44436941cce9370112837582c63885095](https://stellar.expert/explorer/testnet/tx/2b52fbb4253aef87526c7df9527fa7f44436941cce9370112837582c63885095) |
| 40 | Superseded attempt | `sac_authorize.v11_market_pool` | [d3b55160035d3ad9e101d5390e8fba4fb2d39d17fdacb63e356afe21b07548d4](https://stellar.expert/explorer/testnet/tx/d3b55160035d3ad9e101d5390e8fba4fb2d39d17fdacb63e356afe21b07548d4) |
| 41 | Superseded attempt | `permissioning.grant_account.v11_market_pool` | [76f0bab886a1c221916c8f19de45a3cba7f9c774d4f63522a303b9728f514439](https://stellar.expert/explorer/testnet/tx/76f0bab886a1c221916c8f19de45a3cba7f9c774d4f63522a303b9728f514439) |
| 42 | Superseded attempt | `sac_authorize.v11_recovery_escrow` | [d380c473e48c3ce89d548c698f4b85871f930e9fd5b75d771bcfce60e437f53c](https://stellar.expert/explorer/testnet/tx/d380c473e48c3ce89d548c698f4b85871f930e9fd5b75d771bcfce60e437f53c) |
| 43 | Superseded attempt | `permissioning.grant_account.v11_recovery_escrow` | [9d7818e481489a55ad900fc2a5faa7fb80bb7e2efe89919415e9aab3ed6821c0](https://stellar.expert/explorer/testnet/tx/9d7818e481489a55ad900fc2a5faa7fb80bb7e2efe89919415e9aab3ed6821c0) |
| 44 | Superseded attempt | `permissioning.grant_asset.v11_market_pool.v11_pt_token` | [604594485077e54421e99a985d426edfe7d2ee8f0d175b51a3bb4e6fef50f575](https://stellar.expert/explorer/testnet/tx/604594485077e54421e99a985d426edfe7d2ee8f0d175b51a3bb4e6fef50f575) |
| 45 | Superseded attempt | `permissioning.grant_asset.v11_market_pool.v11_yt_token` | [ecebf39ff2a507b1549cd2f538b22b8b51212b910120cc3a446e2418a62da117](https://stellar.expert/explorer/testnet/tx/ecebf39ff2a507b1549cd2f538b22b8b51212b910120cc3a446e2418a62da117) |
| 46 | Superseded attempt | `permissioning.grant_asset.v11_recovery_escrow.v11_pt_token` | [b6a80c657c27ac1683a7251910d7e57db8f71d3353d46d023b516ed54243ea19](https://stellar.expert/explorer/testnet/tx/b6a80c657c27ac1683a7251910d7e57db8f71d3353d46d023b516ed54243ea19) |
| 47 | Superseded attempt | `permissioning.grant_asset.v11_recovery_escrow.v11_yt_token` | [3e1fc71cc2e85f4c751a7442e4beb03aecc8b9dba9cc19024d0e4fed207f4aca](https://stellar.expert/explorer/testnet/tx/3e1fc71cc2e85f4c751a7442e4beb03aecc8b9dba9cc19024d0e4fed207f4aca) |
| 48 | Shared infrastructure | `trustline.v11_treasury.STA` | [cf093500ede3d05ac411442e2fbfc1cf199df06eeeb076e43601a3c67697edd9](https://stellar.expert/explorer/testnet/tx/cf093500ede3d05ac411442e2fbfc1cf199df06eeeb076e43601a3c67697edd9) |
| 49 | Shared infrastructure | `trustline.v11_treasury.STA (first attempt)` | [39193457202546ddb2eed0ec5117d7f39932694df83452d03af2a3d1b93e917f](https://stellar.expert/explorer/testnet/tx/39193457202546ddb2eed0ec5117d7f39932694df83452d03af2a3d1b93e917f) |
| 50 | Shared infrastructure | `auth.v11_treasury.STA` | [43d6b4fdcf36e5a17805cb778354c9dcf38df039ebc041a4b1fd74beee5b78b7](https://stellar.expert/explorer/testnet/tx/43d6b4fdcf36e5a17805cb778354c9dcf38df039ebc041a4b1fd74beee5b78b7) |
| 51 | Shared infrastructure | `sac_authorize.v11_treasury` | [b221335067faf6981b30e5732ae4ba950cf7d31060c84e379673f258a48c519b](https://stellar.expert/explorer/testnet/tx/b221335067faf6981b30e5732ae4ba950cf7d31060c84e379673f258a48c519b) |
| 52 | Shared infrastructure | `trustline.v11-alice.STA` | [929d9a462bef7ccc81626df3c45a9806a9364a453dcac27526646cb685d329fb](https://stellar.expert/explorer/testnet/tx/929d9a462bef7ccc81626df3c45a9806a9364a453dcac27526646cb685d329fb) |
| 53 | Shared infrastructure | `auth.v11-alice.STA` | [9a1641f7d373699c3aca8c843171e62803500661f1c7597abe1869e209ae59c5](https://stellar.expert/explorer/testnet/tx/9a1641f7d373699c3aca8c843171e62803500661f1c7597abe1869e209ae59c5) |
| 54 | Shared infrastructure | `fund.v11-alice.STA` | [5d38dc07e1d68eaf00c71bf460d38ee6a95a10c08002692230b872f6335194d9](https://stellar.expert/explorer/testnet/tx/5d38dc07e1d68eaf00c71bf460d38ee6a95a10c08002692230b872f6335194d9) |
| 55 | Shared infrastructure | `trustline.v11-bob.STA` | [d15b69bb5b26a42d90eb95d6e09d5ac9041f7781c1f211646e78fcc8c617ad1a](https://stellar.expert/explorer/testnet/tx/d15b69bb5b26a42d90eb95d6e09d5ac9041f7781c1f211646e78fcc8c617ad1a) |
| 56 | Shared infrastructure | `auth.v11-bob.STA` | [122a36ae6a84e75c0574c74448b9165fbf101ddcd8d7d72562732c908591b6f5](https://stellar.expert/explorer/testnet/tx/122a36ae6a84e75c0574c74448b9165fbf101ddcd8d7d72562732c908591b6f5) |
| 57 | Shared infrastructure | `fund.v11-bob.STA` | [6076b533d85e1f138b96b2f3751fe1a78815628a5726b759b621ae07b5c9296e](https://stellar.expert/explorer/testnet/tx/6076b533d85e1f138b96b2f3751fe1a78815628a5726b759b621ae07b5c9296e) |
| 58 | Shared infrastructure | `trustline.v11-carol.STA` | [162d5ac4e9d8ec794c7a9b63be79caf50ceeaca4ae90e34c520539b5b68dc1e7](https://stellar.expert/explorer/testnet/tx/162d5ac4e9d8ec794c7a9b63be79caf50ceeaca4ae90e34c520539b5b68dc1e7) |
| 59 | Shared infrastructure | `auth.v11-carol.STA` | [b965f7e55e7d2cc56dbfd06073977e86e8a5436484d67803df69604a4ad21f2a](https://stellar.expert/explorer/testnet/tx/b965f7e55e7d2cc56dbfd06073977e86e8a5436484d67803df69604a4ad21f2a) |
| 60 | Shared infrastructure | `fund.v11-carol.STA` | [2f0248cc67ccad5ecb7d18b5d35ea7c6436e67a80de326f11c8a15561d36eb02](https://stellar.expert/explorer/testnet/tx/2f0248cc67ccad5ecb7d18b5d35ea7c6436e67a80de326f11c8a15561d36eb02) |
| 61 | Shared infrastructure | `permissioning.grant_account.v11-alice` | [9545a5d1e4abb4ba9e946e3379a48eb0b7cbc3c8689e1a89637eb232d9727e8d](https://stellar.expert/explorer/testnet/tx/9545a5d1e4abb4ba9e946e3379a48eb0b7cbc3c8689e1a89637eb232d9727e8d) |
| 62 | Shared infrastructure | `permissioning.grant_asset.v11-alice.v11_pt_token` | [ddb3bbfe0b6d379f2ed80abd9ba9f560a08e0c329c185baf1b8d2c92cc348c2c](https://stellar.expert/explorer/testnet/tx/ddb3bbfe0b6d379f2ed80abd9ba9f560a08e0c329c185baf1b8d2c92cc348c2c) |
| 63 | Shared infrastructure | `permissioning.grant_asset.v11-alice.v11_yt_token` | [df56451406e636551d35dead71bba5a5e41b8764e6cde7ad17854ef368f063bb](https://stellar.expert/explorer/testnet/tx/df56451406e636551d35dead71bba5a5e41b8764e6cde7ad17854ef368f063bb) |
| 64 | Shared infrastructure | `permissioning.grant_account.v11-bob` | [c94164236b02b0565744b1d08014d00f76fa481d092d34bff86678d76bb6cdc9](https://stellar.expert/explorer/testnet/tx/c94164236b02b0565744b1d08014d00f76fa481d092d34bff86678d76bb6cdc9) |
| 65 | Shared infrastructure | `permissioning.grant_asset.v11-bob.v11_pt_token` | [d94e47f6b05f0e5dcab71fdeebe3fa3973c37ba810ce4a0caaad99358653d86c](https://stellar.expert/explorer/testnet/tx/d94e47f6b05f0e5dcab71fdeebe3fa3973c37ba810ce4a0caaad99358653d86c) |
| 66 | Shared infrastructure | `permissioning.grant_asset.v11-bob.v11_yt_token` | [226a0ec2ea24a7ff2ae61b3617667eafc3c11e6ec1665c40ba75da8815e89713](https://stellar.expert/explorer/testnet/tx/226a0ec2ea24a7ff2ae61b3617667eafc3c11e6ec1665c40ba75da8815e89713) |
| 67 | Shared infrastructure | `permissioning.grant_account.v11-carol` | [a83585f4c817025cdafce2ae428339c09d15ddb00a6d95f70a31a7e87e2a78ea](https://stellar.expert/explorer/testnet/tx/a83585f4c817025cdafce2ae428339c09d15ddb00a6d95f70a31a7e87e2a78ea) |
| 68 | Shared infrastructure | `permissioning.grant_asset.v11-carol.v11_pt_token` | [c83945e5ba3c150ca6c89acc8a7426d7e0c7f0f27065661d8aee4cdfa657c2b3](https://stellar.expert/explorer/testnet/tx/c83945e5ba3c150ca6c89acc8a7426d7e0c7f0f27065661d8aee4cdfa657c2b3) |
| 69 | Shared infrastructure | `permissioning.grant_asset.v11-carol.v11_yt_token` | [11306b2e5810554f8e8e7668e27c4ed119559c4be0e2283e896fd463dd394ef2](https://stellar.expert/explorer/testnet/tx/11306b2e5810554f8e8e7668e27c4ed119559c4be0e2283e896fd463dd394ef2) |
| 70 | Shared infrastructure | `oracle.set_reference_value.pre_mint_alice` | [b0e2e8ccaa2fe492dbeb2c26a63c09a4f392876dec6a58a6dd91fcc60a4a5602](https://stellar.expert/explorer/testnet/tx/b0e2e8ccaa2fe492dbeb2c26a63c09a4f392876dec6a58a6dd91fcc60a4a5602) |
| 71 | Superseded attempt | `sy.deposit.alice` | [216408cef79f429632d1d0c26942448e8cefcd816b62523cdc455ae3e3868384](https://stellar.expert/explorer/testnet/tx/216408cef79f429632d1d0c26942448e8cefcd816b62523cdc455ae3e3868384) |
| 72 | Superseded attempt | `m2_pt_token.deploy` | [b744f3affea0dd067dd63e8ed218edc6d936897f3d261ce924ba9ddacfb7b85a](https://stellar.expert/explorer/testnet/tx/b744f3affea0dd067dd63e8ed218edc6d936897f3d261ce924ba9ddacfb7b85a) |
| 73 | Superseded attempt | `m2_pt_token.initialize` | [b8cfbc45aebccb26bfed8a4153ee97c4e54a741e728c580e480608aec9222ec8](https://stellar.expert/explorer/testnet/tx/b8cfbc45aebccb26bfed8a4153ee97c4e54a741e728c580e480608aec9222ec8) |
| 74 | Superseded attempt | `m2_yt_token.deploy` | [f34b4adf8636895a98ecd3e497f2480f4c3eddee5dd235704bd581e2d7676599](https://stellar.expert/explorer/testnet/tx/f34b4adf8636895a98ecd3e497f2480f4c3eddee5dd235704bd581e2d7676599) |
| 75 | Superseded attempt | `m2_yt_token.initialize` | [583fab2ad162099561940c5c96d567cbd848770498f6076e19580abe0757a3e5](https://stellar.expert/explorer/testnet/tx/583fab2ad162099561940c5c96d567cbd848770498f6076e19580abe0757a3e5) |
| 76 | Superseded attempt | `m2_market_config.deploy` | [6e0759c958de10ad72ded07fb22a5d2d76baa899e5c3c5de00d86da378fb7b9e](https://stellar.expert/explorer/testnet/tx/6e0759c958de10ad72ded07fb22a5d2d76baa899e5c3c5de00d86da378fb7b9e) |
| 77 | Superseded attempt | `m2_market_config.initialize` | [d369a44df076f898885deb8cef29b132c96f337e8860f87eb19f3a882107d5cf](https://stellar.expert/explorer/testnet/tx/d369a44df076f898885deb8cef29b132c96f337e8860f87eb19f3a882107d5cf) |
| 78 | Superseded attempt | `m2_principal_manager.deploy` | [cd823691aab94156f09c31666bf70ddf019c65282baa46b7eedac4731ebcaf7a](https://stellar.expert/explorer/testnet/tx/cd823691aab94156f09c31666bf70ddf019c65282baa46b7eedac4731ebcaf7a) |
| 79 | Superseded attempt | `m2_principal_manager.initialize` | [5978a42c525f8445803be908ebd65b6e2e9444aba65024459a2d797b6ba68442](https://stellar.expert/explorer/testnet/tx/5978a42c525f8445803be908ebd65b6e2e9444aba65024459a2d797b6ba68442) |
| 80 | Superseded attempt | `m2_pt_token.set_minter` | [ba5a71dde5a860e5f658234bba8fd15ce209911d70298c336dcefd417e0e5db9](https://stellar.expert/explorer/testnet/tx/ba5a71dde5a860e5f658234bba8fd15ce209911d70298c336dcefd417e0e5db9) |
| 81 | Superseded attempt | `m2_yt_token.set_minter` | [41b22448c2112f65d9da2e80319ed042c4b57d9efc031c375d9e68d8f2442b98](https://stellar.expert/explorer/testnet/tx/41b22448c2112f65d9da2e80319ed042c4b57d9efc031c375d9e68d8f2442b98) |
| 82 | Superseded attempt | `m2_principal_manager.set_risk_control` | [eb342e801563ae2901da00e00153b1a0f1ca2918ad87e8f48ea590cff83b607a](https://stellar.expert/explorer/testnet/tx/eb342e801563ae2901da00e00153b1a0f1ca2918ad87e8f48ea590cff83b607a) |
| 83 | Superseded attempt | `m2_risk_control.add_consumer.pm` | [2a70548ee0e8e14673ffcf6982e09e6dd81e96004eb1bec2d2a95d940f1f77b9](https://stellar.expert/explorer/testnet/tx/2a70548ee0e8e14673ffcf6982e09e6dd81e96004eb1bec2d2a95d940f1f77b9) |
| 84 | Superseded attempt | `m2_market_pool.deploy` | [5fe9cfda727226df50d2296bb949a9f8bafa21ba14d2545bbefabca5e512c4c4](https://stellar.expert/explorer/testnet/tx/5fe9cfda727226df50d2296bb949a9f8bafa21ba14d2545bbefabca5e512c4c4) |
| 85 | Superseded attempt | `m2_market_pool.initialize` | [bb8bbe5fd8f09d3cc4f16d448e8d43757935be7fd9dde4e7184837674ca2f263](https://stellar.expert/explorer/testnet/tx/bb8bbe5fd8f09d3cc4f16d448e8d43757935be7fd9dde4e7184837674ca2f263) |
| 86 | Superseded attempt | `m2_recovery_escrow.deploy` | [a46dc6da31b30ef7be965c905d3bee1f543f8707186bb8398cf25f8769ac29c4](https://stellar.expert/explorer/testnet/tx/a46dc6da31b30ef7be965c905d3bee1f543f8707186bb8398cf25f8769ac29c4) |
| 87 | Superseded attempt | `m2_recovery_escrow.initialize` | [0cd1763ab88b526bcbb352bbd451ddb960ac0fea6d8c03773a4265ed8e560dd9](https://stellar.expert/explorer/testnet/tx/0cd1763ab88b526bcbb352bbd451ddb960ac0fea6d8c03773a4265ed8e560dd9) |
| 88 | Final market | `m4_sy_wrapper.deploy` | [a06454262a7663c8690b2180b8ada6169fbd15325cd43bb97c248372fc6e45a4](https://stellar.expert/explorer/testnet/tx/a06454262a7663c8690b2180b8ada6169fbd15325cd43bb97c248372fc6e45a4) |
| 89 | Final market | `m4_sy_wrapper.initialize` | [ca719d10eb4f68cb2a31e9148a20ebc49cfd769bc0d63719249313e654763b6c](https://stellar.expert/explorer/testnet/tx/ca719d10eb4f68cb2a31e9148a20ebc49cfd769bc0d63719249313e654763b6c) |
| 90 | Final market | `m4_sy_wrapper.set_risk_control` | [92801839048f727d06850c8972ac6f23fdcd7d0ddf0a65351263a5868d124095](https://stellar.expert/explorer/testnet/tx/92801839048f727d06850c8972ac6f23fdcd7d0ddf0a65351263a5868d124095) |
| 91 | Final market | `m4_risk_control.add_consumer.sy` | [a192bee1c65c9c84d9323b8cdd39f0823c4f1b6678f94506efea822fb46c4ed9](https://stellar.expert/explorer/testnet/tx/a192bee1c65c9c84d9323b8cdd39f0823c4f1b6678f94506efea822fb46c4ed9) |
| 92 | Final market | `sac_authorize.m4_sy_wrapper` | [a6cab86bc7036602e3136e71850516b7569b9ea28e3d60745f9a6d65321cb705](https://stellar.expert/explorer/testnet/tx/a6cab86bc7036602e3136e71850516b7569b9ea28e3d60745f9a6d65321cb705) |
| 93 | Final market | `permissioning.grant_account.m4_sy_wrapper` | [bca76be3c66e3e3230fda925d56da3afc6a00026d2e88ab15b0f91be98a6529c](https://stellar.expert/explorer/testnet/tx/bca76be3c66e3e3230fda925d56da3afc6a00026d2e88ab15b0f91be98a6529c) |
| 94 | Final market | `m4_pt_token.deploy` | [b0fa8bbf9ca759ec8aaf80c2b46f52332f57fcfe1796c04df9700a316c1d2076](https://stellar.expert/explorer/testnet/tx/b0fa8bbf9ca759ec8aaf80c2b46f52332f57fcfe1796c04df9700a316c1d2076) |
| 95 | Final market | `m4_pt_token.initialize` | [80d26166c63f1f4f3032cb52c5abebc26cb3079f36ec93ed7b286abccaa44cd4](https://stellar.expert/explorer/testnet/tx/80d26166c63f1f4f3032cb52c5abebc26cb3079f36ec93ed7b286abccaa44cd4) |
| 96 | Final market | `m4_yt_token.deploy` | [0fac220ccfc6093a737a00ae009fd9ad7d911031797397d29eb233d8791aabf6](https://stellar.expert/explorer/testnet/tx/0fac220ccfc6093a737a00ae009fd9ad7d911031797397d29eb233d8791aabf6) |
| 97 | Final market | `m4_yt_token.initialize` | [5b2d48df77b5523629276bba00c60b052911cc3388cccf3b4e4dfdca4aca5d3d](https://stellar.expert/explorer/testnet/tx/5b2d48df77b5523629276bba00c60b052911cc3388cccf3b4e4dfdca4aca5d3d) |
| 98 | Final market | `m4_market_config.deploy` | [3f89c5ea052195471b19f2159b326f1790f9e778ab98b746fc3d05290e9127cf](https://stellar.expert/explorer/testnet/tx/3f89c5ea052195471b19f2159b326f1790f9e778ab98b746fc3d05290e9127cf) |
| 99 | Final market | `m4_market_config.initialize` | [5fe6d30ed33cde6093862bcdf88504b5412727455e3304e46cfb08ae9b29faee](https://stellar.expert/explorer/testnet/tx/5fe6d30ed33cde6093862bcdf88504b5412727455e3304e46cfb08ae9b29faee) |
| 100 | Final market | `m4_principal_manager.deploy` | [7ef536a6f6d94f7e54fb29285299e135babc1d4d6d355e338a441f32de742175](https://stellar.expert/explorer/testnet/tx/7ef536a6f6d94f7e54fb29285299e135babc1d4d6d355e338a441f32de742175) |
| 101 | Final market | `m4_principal_manager.initialize` | [28b6df6b290c091d2836431e53fdc50f195bac7fe5e68b774343ba70f2d9e627](https://stellar.expert/explorer/testnet/tx/28b6df6b290c091d2836431e53fdc50f195bac7fe5e68b774343ba70f2d9e627) |
| 102 | Final market | `m4_pt_token.set_minter` | [e356133c85c95eed9df8fd3c7770761b8c16d400ab8d1e57399e74ed473af535](https://stellar.expert/explorer/testnet/tx/e356133c85c95eed9df8fd3c7770761b8c16d400ab8d1e57399e74ed473af535) |
| 103 | Final market | `m4_yt_token.set_minter` | [52262344a20a39f1e0bcb6a90f73a500c4490e6039c2a529b39e72ebca1e6e79](https://stellar.expert/explorer/testnet/tx/52262344a20a39f1e0bcb6a90f73a500c4490e6039c2a529b39e72ebca1e6e79) |
| 104 | Final market | `m4_principal_manager.set_risk_control` | [1687663b98e68a86c3fe52f6e3ea4c3691fd588485b75f90da2d052c5929c139](https://stellar.expert/explorer/testnet/tx/1687663b98e68a86c3fe52f6e3ea4c3691fd588485b75f90da2d052c5929c139) |
| 105 | Final market | `m4_risk_control.add_consumer.pm` | [436e7d0bd9f988a4085115d252db1b687f7bd1aff7fc575f5d0e30d931ad40aa](https://stellar.expert/explorer/testnet/tx/436e7d0bd9f988a4085115d252db1b687f7bd1aff7fc575f5d0e30d931ad40aa) |
| 106 | Final market | `m4_market_pool.deploy` | [b267f153edecd7934dd0b0d0ef7601a1be237c88f8bfb433156cada3d813a094](https://stellar.expert/explorer/testnet/tx/b267f153edecd7934dd0b0d0ef7601a1be237c88f8bfb433156cada3d813a094) |
| 107 | Final market | `m4_market_pool.initialize` | [ff34b7498adcc911fa8b8edbebd2ef6c73fa0fa218c5ccffd60feeb0dea46b19](https://stellar.expert/explorer/testnet/tx/ff34b7498adcc911fa8b8edbebd2ef6c73fa0fa218c5ccffd60feeb0dea46b19) |
| 108 | Final market | `m4_recovery_escrow.deploy` | [bf887ced23993375df0f1cd2c123f5e886e6fb7db47e83e1d61dc7a58199652d](https://stellar.expert/explorer/testnet/tx/bf887ced23993375df0f1cd2c123f5e886e6fb7db47e83e1d61dc7a58199652d) |
| 109 | Final market | `m4_recovery_escrow.initialize` | [b3f3c93f0b0b049a4e69375374f61cde9fa6eb9b63344a6f055d7523c953e346](https://stellar.expert/explorer/testnet/tx/b3f3c93f0b0b049a4e69375374f61cde9fa6eb9b63344a6f055d7523c953e346) |
| 110 | Final market | `m4_sy_wrapper.set_recovery_escrow` | [e9162c64726292f9624a90b04529aac4b83d4b8d8dba2b3997aab66169cf303f](https://stellar.expert/explorer/testnet/tx/e9162c64726292f9624a90b04529aac4b83d4b8d8dba2b3997aab66169cf303f) |
| 111 | Final market | `m4_pt_token.set_recovery_escrow` | [854eaef371fde03b0e4aea008f3dcd4ca0de60bbb7d2c672e89d78649c49e3aa](https://stellar.expert/explorer/testnet/tx/854eaef371fde03b0e4aea008f3dcd4ca0de60bbb7d2c672e89d78649c49e3aa) |
| 112 | Final market | `m4_yt_token.set_recovery_escrow` | [0477fb5cb82d4073391cd8d24d29648f5a57d2904d28dfd222e5f68cab2eb01a](https://stellar.expert/explorer/testnet/tx/0477fb5cb82d4073391cd8d24d29648f5a57d2904d28dfd222e5f68cab2eb01a) |
| 113 | Final market | `m4_market_pool.set_recovery_escrow` | [9f96dd1e182e4a183be956184e9be4044be48380b24cd490b42e800422e42a94](https://stellar.expert/explorer/testnet/tx/9f96dd1e182e4a183be956184e9be4044be48380b24cd490b42e800422e42a94) |
| 114 | Final market | `m4_router.deploy` | [e6c1670005328ef4eb9a8c1851c03535d4677a14da8d36f1398adf108d65c5c8](https://stellar.expert/explorer/testnet/tx/e6c1670005328ef4eb9a8c1851c03535d4677a14da8d36f1398adf108d65c5c8) |
| 115 | Final market | `m4_router.initialize` | [653d903b385327226547a4438b80ddc6f8508ca35e5ae554bd49123838cfa4d5](https://stellar.expert/explorer/testnet/tx/653d903b385327226547a4438b80ddc6f8508ca35e5ae554bd49123838cfa4d5) |
| 116 | Final market | `m4_router.register_market` | [6742da01a47d343b53db6f71d1c1b4fa60ad5c0b4539f62872870c8f36244c9b](https://stellar.expert/explorer/testnet/tx/6742da01a47d343b53db6f71d1c1b4fa60ad5c0b4539f62872870c8f36244c9b) |
| 117 | Final market | `sac_authorize.m4_principal_manager` | [0910e861d76240413e7ce293522d6bd133f5263b4c9ba67776c9fd1e35ef0f38](https://stellar.expert/explorer/testnet/tx/0910e861d76240413e7ce293522d6bd133f5263b4c9ba67776c9fd1e35ef0f38) |
| 118 | Final market | `permissioning.grant_account.m4_principal_manager` | [6728cffe3203c064787d26215a1e487ff15462aeeeb5ac803cc8ec8d39cbc256](https://stellar.expert/explorer/testnet/tx/6728cffe3203c064787d26215a1e487ff15462aeeeb5ac803cc8ec8d39cbc256) |
| 119 | Final market | `sac_authorize.m4_market_pool` | [0a7da1be7181bfe2bf2374c1c2716096416eefdac8a32b607e3fd7243aad0136](https://stellar.expert/explorer/testnet/tx/0a7da1be7181bfe2bf2374c1c2716096416eefdac8a32b607e3fd7243aad0136) |
| 120 | Final market | `permissioning.grant_account.m4_market_pool` | [26f4cd7b8085851775f0e8041e4feebe77d04d252f9c006c0426a1a5829d5970](https://stellar.expert/explorer/testnet/tx/26f4cd7b8085851775f0e8041e4feebe77d04d252f9c006c0426a1a5829d5970) |
| 121 | Final market | `sac_authorize.m4_recovery_escrow` | [cbea4dbc42e44922a8b8d12425378656d0f2ea4efff1f35a60a8652badc50526](https://stellar.expert/explorer/testnet/tx/cbea4dbc42e44922a8b8d12425378656d0f2ea4efff1f35a60a8652badc50526) |
| 122 | Final market | `permissioning.grant_account.m4_recovery_escrow` | [7975b959bf4b19d55c090b567f3caf31555cf8c8162c08486aba8467562e7eec](https://stellar.expert/explorer/testnet/tx/7975b959bf4b19d55c090b567f3caf31555cf8c8162c08486aba8467562e7eec) |
| 123 | Final market | `permissioning.grant_asset.m4_market_pool.m4_pt_token` | [202169257688fc3d9afc7eb5da8fe9f237430ca7eddbc94069c816ff55f48d62](https://stellar.expert/explorer/testnet/tx/202169257688fc3d9afc7eb5da8fe9f237430ca7eddbc94069c816ff55f48d62) |
| 124 | Final market | `permissioning.grant_asset.m4_market_pool.m4_yt_token` | [a09e6e8a69b316bd3723428c7265678a81f4969623e530e5e8aba299ffd688e9](https://stellar.expert/explorer/testnet/tx/a09e6e8a69b316bd3723428c7265678a81f4969623e530e5e8aba299ffd688e9) |
| 125 | Final market | `permissioning.grant_asset.m4_recovery_escrow.m4_pt_token` | [6be5db7e0843344798e6f3cae9b817228ab401e466cc36206c358b877a1d096c](https://stellar.expert/explorer/testnet/tx/6be5db7e0843344798e6f3cae9b817228ab401e466cc36206c358b877a1d096c) |
| 126 | Final market | `permissioning.grant_asset.m4_recovery_escrow.m4_yt_token` | [22167387fa7b025a19629b4cf9870ab6a00231b188f898537c61b6588d5db3a9](https://stellar.expert/explorer/testnet/tx/22167387fa7b025a19629b4cf9870ab6a00231b188f898537c61b6588d5db3a9) |
| 127 | Shared infrastructure | `permissioning.grant_account.alice` | [a31b3410ef2a9d4b74f1a53fa29cc5237388d033982ff38bde50c1ca11a7c819](https://stellar.expert/explorer/testnet/tx/a31b3410ef2a9d4b74f1a53fa29cc5237388d033982ff38bde50c1ca11a7c819) |
| 128 | Shared infrastructure | `permissioning.grant_asset.alice.m4_pt_token` | [bce348130bfd74002f8abd81318c48585b1c6e4e70f012c87b0df8a9784e85dc](https://stellar.expert/explorer/testnet/tx/bce348130bfd74002f8abd81318c48585b1c6e4e70f012c87b0df8a9784e85dc) |
| 129 | Shared infrastructure | `permissioning.grant_asset.alice.m4_yt_token` | [ce51e2a0a802e42129caf51c671196e7e2da273c34d02f4f5fddba33b945868b](https://stellar.expert/explorer/testnet/tx/ce51e2a0a802e42129caf51c671196e7e2da273c34d02f4f5fddba33b945868b) |
| 130 | Shared infrastructure | `permissioning.grant_account.bob` | [03b77b2506f8dd6150e21a891e3e5db420e496d60ab192e5e3f2571d43bb132d](https://stellar.expert/explorer/testnet/tx/03b77b2506f8dd6150e21a891e3e5db420e496d60ab192e5e3f2571d43bb132d) |
| 131 | Shared infrastructure | `permissioning.grant_asset.bob.m4_pt_token` | [bf9ec7d3b35c7efeeb4d533a488a9591ea60dc3c5086b71d627dd31af6f24c1f](https://stellar.expert/explorer/testnet/tx/bf9ec7d3b35c7efeeb4d533a488a9591ea60dc3c5086b71d627dd31af6f24c1f) |
| 132 | Shared infrastructure | `permissioning.grant_asset.bob.m4_yt_token` | [ce9804ca2766b749627918917f4166e6c7c5989be644c4a0fe48062f32ba6e95](https://stellar.expert/explorer/testnet/tx/ce9804ca2766b749627918917f4166e6c7c5989be644c4a0fe48062f32ba6e95) |
| 133 | Shared infrastructure | `permissioning.grant_account.carol` | [321d25b7f51599273e0d5250e1d20538782d948977e575c85b20b7c085d2d138](https://stellar.expert/explorer/testnet/tx/321d25b7f51599273e0d5250e1d20538782d948977e575c85b20b7c085d2d138) |
| 134 | Shared infrastructure | `permissioning.grant_asset.carol.m4_pt_token` | [becded8b3b8bc4247a188e3034e173d90d20a561458aba89a4312fb65948e3ea](https://stellar.expert/explorer/testnet/tx/becded8b3b8bc4247a188e3034e173d90d20a561458aba89a4312fb65948e3ea) |
| 135 | Shared infrastructure | `permissioning.grant_asset.carol.m4_yt_token` | [c5fb31f4958bbc7a17029b84b23452e9af307bfe4c4f88b9bd834b3806ca32c0](https://stellar.expert/explorer/testnet/tx/c5fb31f4958bbc7a17029b84b23452e9af307bfe4c4f88b9bd834b3806ca32c0) |
| 136 | Shared infrastructure | `oracle.set_reference_value.pre_mint` | [a361ec9d4b62f297e9a27af025545c78404691ba9d10c8c57b760502671b6dff](https://stellar.expert/explorer/testnet/tx/a361ec9d4b62f297e9a27af025545c78404691ba9d10c8c57b760502671b6dff) |
| 137 | Final market | `sy.deposit.alice` | [56a857fbec24a3e3ea1828447099ac637c3365708efcf45b966fcaf393e2b977](https://stellar.expert/explorer/testnet/tx/56a857fbec24a3e3ea1828447099ac637c3365708efcf45b966fcaf393e2b977) |
| 138 | Final market | `pm.mint.alice` | [fd90a3294c4ff78cf146383045e37149a516da8823b12b3c1bcf12c066efec0b](https://stellar.expert/explorer/testnet/tx/fd90a3294c4ff78cf146383045e37149a516da8823b12b3c1bcf12c066efec0b) |
| 139 | Final market | `sy.deposit.bob` | [d19abe956b6494a480b41a678cc94c6a3963e9e3eed340a015c294813d71e7bf](https://stellar.expert/explorer/testnet/tx/d19abe956b6494a480b41a678cc94c6a3963e9e3eed340a015c294813d71e7bf) |
| 140 | Final market | `pm.mint.bob` | [b5adb4cd222b8f598c107784ca104cfaecd65316d8500871bee62029076b25e8](https://stellar.expert/explorer/testnet/tx/b5adb4cd222b8f598c107784ca104cfaecd65316d8500871bee62029076b25e8) |
| 141 | Final market | `sy.deposit.carol` | [680fc41ad11e660b29cb9ca3f57a2fe711bf257bb42ad08ee2d5ba3ac8331de6](https://stellar.expert/explorer/testnet/tx/680fc41ad11e660b29cb9ca3f57a2fe711bf257bb42ad08ee2d5ba3ac8331de6) |
| 142 | Final market | `pm.mint.carol` | [ed1e4405669836eb1f3bd6f56bfb4e5ace2cd430535497bfee6e5d3db1f8b76a](https://stellar.expert/explorer/testnet/tx/ed1e4405669836eb1f3bd6f56bfb4e5ace2cd430535497bfee6e5d3db1f8b76a) |
| 143 | Final market | `pool.add_liquidity.carol` | [e2c91b00540ad5bf2f683e827e711c74e1fae0d35a6d79c9b636d05cf064f489](https://stellar.expert/explorer/testnet/tx/e2c91b00540ad5bf2f683e827e711c74e1fae0d35a6d79c9b636d05cf064f489) |
| 144 | Final market | `pool.swap_pt_for_sy.alice` | [018557459b8e00da68fb8232718269f050c64f36711188aa95e5d6d2340d94bf](https://stellar.expert/explorer/testnet/tx/018557459b8e00da68fb8232718269f050c64f36711188aa95e5d6d2340d94bf) |
| 145 | Final market | `pool.swap_sy_for_pt.carol` | [9c9e4e29284a739cb603db5cd72cecb9abd2cdc25a9591f185745266f1035624](https://stellar.expert/explorer/testnet/tx/9c9e4e29284a739cb603db5cd72cecb9abd2cdc25a9591f185745266f1035624) |
| 146 | Final market | `router.swap_sy_for_yt.carol` | [d0d56a6a7ac13e3e1394e4637f54748101fb147b875faae9e0cd581873ec03d5](https://stellar.expert/explorer/testnet/tx/d0d56a6a7ac13e3e1394e4637f54748101fb147b875faae9e0cd581873ec03d5) |
| 147 | Shared infrastructure | `oracle.set_reference_value.yield_step` | [ecc2af472413d0281a040012f8aee3465a4a9526a5cc2ecd3ae803eb251dd0cc](https://stellar.expert/explorer/testnet/tx/ecc2af472413d0281a040012f8aee3465a4a9526a5cc2ecd3ae803eb251dd0cc) |
| 148 | Final market | `pm.claim_yield.alice` | [f1991661956c6d11430a60cfd655b2bb9ecc951390522685554dc9c8caef8530](https://stellar.expert/explorer/testnet/tx/f1991661956c6d11430a60cfd655b2bb9ecc951390522685554dc9c8caef8530) |
| 149 | Final market | `sac_deauthorize.bob` | [f8fab2b81334b08dd028841dd1976a2dcd870b1fd95c5a9041db1eceaaaaf7a5](https://stellar.expert/explorer/testnet/tx/f8fab2b81334b08dd028841dd1976a2dcd870b1fd95c5a9041db1eceaaaaf7a5) |
| 150 | Final market | `escrow.seize_all_positions.bob` | [c9e910c7b1f0ab6e9e88ed8d988121531b40a7c282746effc6a5830e8556751e](https://stellar.expert/explorer/testnet/tx/c9e910c7b1f0ab6e9e88ed8d988121531b40a7c282746effc6a5830e8556751e) |
| 151 | Shared infrastructure | `oracle.set_reference_value.post_maturity` | [ae8d265cf8a260cc4995fc02a6e1378a41587d8c7bb03f7c3432af7c708990ef](https://stellar.expert/explorer/testnet/tx/ae8d265cf8a260cc4995fc02a6e1378a41587d8c7bb03f7c3432af7c708990ef) |
| 152 | Final market | `pm.settle_all.m4` | [440756cabb173e0aa1be12dcf71f6fe765089f1935514542d2a0104196a8cb1e](https://stellar.expert/explorer/testnet/tx/440756cabb173e0aa1be12dcf71f6fe765089f1935514542d2a0104196a8cb1e) |
| 153 | Final market | `pm.redeem.alice` | [fbe8cd2291607907ed8c46e4282af1d44e6c98c6339a021be4563800aa64d8c0](https://stellar.expert/explorer/testnet/tx/fbe8cd2291607907ed8c46e4282af1d44e6c98c6339a021be4563800aa64d8c0) |
| 154 | Final market | `pool.remove_liquidity.carol` | [bd7a55bd4dae87eeba2ab4cd429933a01ffdc48d9822ebeda7b22019a40890bb](https://stellar.expert/explorer/testnet/tx/bd7a55bd4dae87eeba2ab4cd429933a01ffdc48d9822ebeda7b22019a40890bb) |
| 155 | Shared infrastructure | `permissioning.grant_account.treasury` | [961924fcf373e229b6333dbc13bbb90acd0d8edaf6be92a7fa60b060e38bcc5b](https://stellar.expert/explorer/testnet/tx/961924fcf373e229b6333dbc13bbb90acd0d8edaf6be92a7fa60b060e38bcc5b) |
| 156 | Final market | `pool.claim_protocol_fees` | [f005dbe552a9272e15bef9eb3e4e480cc085e04ddf863c00375fd412979ed7e5](https://stellar.expert/explorer/testnet/tx/f005dbe552a9272e15bef9eb3e4e480cc085e04ddf863c00375fd412979ed7e5) |
| 157 | Shared infrastructure | `permissioning.grant_account.creator` | [9356fe7f12906cebddd30fd75d4bbf0952a7899b5d05db27ac86b93a6301cb56](https://stellar.expert/explorer/testnet/tx/9356fe7f12906cebddd30fd75d4bbf0952a7899b5d05db27ac86b93a6301cb56) |
| 158 | Final market | `pool.claim_creator_fees` | [71c12aa2bb45b8b50c65cc3ba312592c20edbc8a6684b6aa6077c55200fd279a](https://stellar.expert/explorer/testnet/tx/71c12aa2bb45b8b50c65cc3ba312592c20edbc8a6684b6aa6077c55200fd279a) |
| 159 | Final market | `escrow.finalize_record.bob` | [7e2b07f685ac79efd622b7ea4680b18e700d96caebf9ccd9376fe6d7e8aee8e3](https://stellar.expert/explorer/testnet/tx/7e2b07f685ac79efd622b7ea4680b18e700d96caebf9ccd9376fe6d7e8aee8e3) |

## 8. Verification result

All 159 unique transaction hashes were queried on the Horizon Testnet API on 4 October 2026. All 159 report `successful: true`, and none failed. The contract addresses in section 3 were queried on the Stellar Expert Testnet API, which returned a creation record for each one.

## 9. Reproduction

The deployment was run with the Stellar CLI 26.0.0, from `scratchpad/deploy` in this session, using the helper library in `lib.sh` and the scripts `deploy2.sh`, `deploy3.sh` and `deploy4.sh`, followed by `post.sh`. The contract WASM files were built with `cargo build --workspace --release --target wasm32v1-none`. These scripts are not committed to the repository, so the commands in [DEPLOYMENT.md](https://github.com/principal-Protocol-org/principal/blob/6cf257afb588c63d694f2262d8b289d4809be8ee/docs/DEPLOYMENT.md) are the reference for reproducing the setup.
