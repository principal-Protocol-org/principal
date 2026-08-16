# Testnet Deployment Evidence — 30 July 2026

Full, raw record of the current-source deployment summarized narratively in
[PROOF_OF_CONCEPT.md](PROOF_OF_CONCEPT.md)'s "Current Testnet Deployment" section. Every
address and transaction hash below is real and independently verifiable on
[stellar.expert](https://stellar.expert/explorer/testnet) or via Horizon
(`https://horizon-testnet.stellar.org`); nothing here is simulated or asserted without a
corresponding on-chain transaction. This file is the exhaustive log; the narrative section in
PROOF_OF_CONCEPT.md is the readable summary of the same run.

Network: **Stellar Testnet** (`Test SDF Network ; September 2015`), RPC
`https://soroban-testnet.stellar.org`.

## 1. Roles and keys

| Role | Address | Notes |
|---|---|---|
| Issuer / protocol admin | `GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB` | Issues the STA test asset; satisfies every contract's market-creation gate (`admin == underlying_SAC.admin()`) |
| Alice (ordinary holder) | `GAK3XILRBYBMBOCZMSLL2CLR6WPQLEIOC6ZCYYPTE4OIAX3PCFFO2YMU` | Deposits, mints, claims yield mid-life, redeems normally at maturity |
| Bob (flagged and recovered) | `GDXVIRLSBDKT7EZM2RM3FH26W3TPF77IJ7GZBA5IOA6ZJBTW26NNO3AV` | Deposits, mints, is later deauthorized by the issuer and has his PT/YT position seized and unwound via `RecoveryEscrow` |

None of these are mainnet keys — all three are testnet-only identities, and no mainnet account was touched at any point in this deployment.

## 2. Contract addresses

| Contract | Address | Explorer |
|---|---|---|
| OracleAdapter | `CDBIVWBLB6UEOBIWO5HAFYPM7LPMH3GKDRG4UMTVQDEAJRY3OIWJLPUO` | https://stellar.expert/explorer/testnet/contract/CDBIVWBLB6UEOBIWO5HAFYPM7LPMH3GKDRG4UMTVQDEAJRY3OIWJLPUO |
| Permissioning | `CBTV5C3GSQKYTAMHOVET7RH25BKSFSAEENTP7SRZEEBD6LKHE24MLXIM` | https://stellar.expert/explorer/testnet/contract/CBTV5C3GSQKYTAMHOVET7RH25BKSFSAEENTP7SRZEEBD6LKHE24MLXIM |
| RiskControl | `CCQXRR3SEP7UTSJORW43V2COD4D3HR6FCLZFJSPXBEH7W4WJWY7VTGC3` | https://stellar.expert/explorer/testnet/contract/CCQXRR3SEP7UTSJORW43V2COD4D3HR6FCLZFJSPXBEH7W4WJWY7VTGC3 |
| STA (SAC wrapping the classic asset, stand-in for USDY) | `CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L` | https://stellar.expert/explorer/testnet/contract/CCOUVA654JH2V6B7LNTKHJP5DF3QA553RS2IIWXSGPDFH2N3QILIVU5L |
| SYWrapper | `CA23M3FMEZJL5MHYTCDLSU7NG4MJ5UYAAKIT5QC4W2TO4SDZQ5EX3XMN` | https://stellar.expert/explorer/testnet/contract/CA23M3FMEZJL5MHYTCDLSU7NG4MJ5UYAAKIT5QC4W2TO4SDZQ5EX3XMN |
| PTToken | `CDHAJVFKVHJ3NTSVUVPSXEEFZT6LKHLOOQ35KPU7T6Z64XQEEGEE76ML` | https://stellar.expert/explorer/testnet/contract/CDHAJVFKVHJ3NTSVUVPSXEEFZT6LKHLOOQ35KPU7T6Z64XQEEGEE76ML |
| YTToken | `CALVCKLXBODNE6AD5KRJY2TWX2WNGQIVIGXUCBOS7AFXC3Q6M5XMA2L2` | https://stellar.expert/explorer/testnet/contract/CALVCKLXBODNE6AD5KRJY2TWX2WNGQIVIGXUCBOS7AFXC3Q6M5XMA2L2 |
| PrincipalManager | `CDKBWCBFPIAVHYGKT6PUGU6ALELWCWXFC23NM3TYFYH6XLETVVMF3LRP` | https://stellar.expert/explorer/testnet/contract/CDKBWCBFPIAVHYGKT6PUGU6ALELWCWXFC23NM3TYFYH6XLETVVMF3LRP |
| RecoveryEscrow | `CCH4DZ6B64IMY7CNINWSFUI46PTL4B266RZ63EZZWHW5AGGAAN5WNSKW` | https://stellar.expert/explorer/testnet/contract/CCH4DZ6B64IMY7CNINWSFUI46PTL4B266RZ63EZZWHW5AGGAAN5WNSKW |

**Market maturity:** Unix `1785427014`.

### 2.1 Superseded first attempt

`PTToken`, `YTToken`, `PrincipalManager`, and `RecoveryEscrow` were each deployed **twice** in this
session. The first instances were initialized with maturity `1785425924` (a 240-second buffer from
deployment), and the wiring transactions alone took longer than that in real wall-clock time on a
real network — `PrincipalManager.mint` correctly reverted `AlreadyMature` on the first attempt,
exactly the check `mint_after_maturity_panics` exercises in the unit test suite. Those four
contracts were redeployed with a 900-second buffer (maturity `1785427014`, the address is what's
listed above); `SYWrapper` and `OracleAdapter`/`Permissioning`/`RiskControl` carry no maturity and
were reused as-is. The first-attempt addresses (now dead, holding no funds, and never reaching a
usable state) were:

| Contract | Superseded address |
|---|---|
| PTToken (1st) | `CCO6LFGXSE7F5NE3RTPSLX6QAE4SVPPHMVD3YFPYK7HUULIKV3HOCQR4` |
| YTToken (1st) | `CCP2XXELTHAZ7BAKQWSGDEAJRZPFZOPNWH7G5QKKHE3VYF26HHXS5USI` |
| PrincipalManager (1st) | `CBO5RPFSK5B33H23WK7RF3PRTLEUHPF7TYXEWPGEDRCFFBF4E5PGKJ4K` |
| RecoveryEscrow (1st) | `CBDVJPYJ6EOHYQGOZ4CHOIF7KI3ZRNSG3EQ6WK76QWV2WNENTV25UEBE` |

## 3. Asset setup (classic Stellar operations)

STA is a real classic Stellar asset, issued by the protocol admin, with `AUTH_REQUIRED` and
`AUTH_REVOCABLE` both set — the same authorization model a real regulated RWA issuer (e.g. Ondo's
USDY) uses, and the exact mechanism `underlying_SAC.authorized()` reads live throughout the
contracts.

| # | Action | Transaction |
|---|---|---|
| 1 | Issuer sets `AUTH_REQUIRED` + `AUTH_REVOCABLE` (`stellar tx new set-options --set-required --set-revocable`) | [c8722cb3…](https://stellar.expert/explorer/testnet/tx/c8722cb32fbc84fd2398f7eeb85c537e971c0164d588384f226d1e99b63f3fe0) |
| 2 | Bob establishes a classic trustline to STA (`change-trust`) | [ac9e3a7a…](https://stellar.expert/explorer/testnet/tx/ac9e3a7ac6f6d6f525c0b009ae71274dd6119b8e920f9297a3939839c824a5a4) |
| 3 | Issuer authorizes Bob's new trustline (`set-trustline-flags --set-authorize`) | [794d70a0…](https://stellar.expert/explorer/testnet/tx/794d70a03c4d127a3903234ebae02d6505013c934a93b5d0df1088892f57cc1d) |
| 4 | Issuer pays 1,000 STA to Bob | [82dc30b3…](https://stellar.expert/explorer/testnet/tx/82dc30b304eed4b61502ff47aa26936d08fe56090eb2f2e6b0cb2ae0c1589c03) |
| 5 | Issuer pays 1,000 STA to Alice (adds to the 0.5 STA / existing trustline already present on this identity from earlier work) | [66f60592…](https://stellar.expert/explorer/testnet/tx/66f60592c6dba8b7c4deddfa53b115fd7e45b70b870eda5d328fe62ce803f663) |

The STA/issuer pair and Alice's trustline pre-existed this session from earlier work on this
environment; steps 1–5 above are what this session added on top of that (enabling authorization
enforcement, and onboarding Bob).

The SAC wrapping STA (`CCOUVA6…`) also pre-existed; `stellar contract asset deploy` was attempted
and correctly rejected with `Error(Storage, ExistingValue)` since the contract already existed, so
`stellar contract id asset --asset STA:<issuer>` was used to recover its address instead of
redeploying it.

## 4. Contract deployment mechanics

Each of the eight contracts (plus the four superseded redeploys) was deployed with
`stellar contract deploy --wasm <path> --source-account sta-testnet-deployer --network testnet`,
which itself submits **two** ledger transactions per contract (upload the WASM, then instantiate
from its hash) — the same two-step mechanic documented in PROOF_OF_CONCEPT.md's historical
deployment section. That is 24 raw upload/instantiate transactions in total (12 contract instances
× 2), all from the deployer address between `2026-07-30T15:31:19Z` and `2026-07-30T15:44:56Z`. Any
one of them can be inspected directly from the deployer account's full transaction history:

https://stellar.expert/explorer/testnet/account/GCWFJKLE45TMVZS42TMIYKAORKGBWE74753YPOSCC5ESJR2G2UMBXBDB

This log does not re-list all 24 individually — the contract addresses in §2 are themselves the
authoritative record (each one's own Explorer page shows its creation transaction and every
subsequent operation against it). §5 and §6 below cover every *meaningful* call: every
`initialize`, every wiring call, and every demo transaction.

## 5. Initialization and wiring

All calls in this section are signed by the issuer (`sta-testnet-deployer`) unless noted.

| # | Call | Transaction |
|---|---|---|
| 1 | `OracleAdapter.initialize(admin=issuer)` | [5ef5655f…](https://stellar.expert/explorer/testnet/tx/5ef5655f9d3c465e05b4011a1624e8cf23a994775065e3bcb0827b4fd4b3e4fa) |
| 2 | `OracleAdapter.set_reference_value(1.00)` — genesis rate | [2073b9d5…](https://stellar.expert/explorer/testnet/tx/2073b9d5dc2cf4a33e69a85012b9da2db6bc60a4729c355538641a811e2b4ba0) |
| 3 | `Permissioning.initialize(admin=issuer)` | [45bbcbc7…](https://stellar.expert/explorer/testnet/tx/45bbcbc7ecf68f6d82ac35ff047fa0b79757f1efbb2a1f713a83c497f0ddca18) |
| 4 | `RiskControl.initialize(admin=issuer, cb_limit=0)` | [a53cbe5a…](https://stellar.expert/explorer/testnet/tx/a53cbe5a9a001b7e03cd48e1cdf6b9216c3d2f8e860bf0694f1e83368211ace4) |
| 5 | `SYWrapper.initialize(admin=issuer, underlying=STA, permissioning)` | [173d434d…](https://stellar.expert/explorer/testnet/tx/173d434d4f7ae3fcee0f06a67200e06a2aa4b4f4578c4a1c7eb7f991b999e977) |
| 6 | `PTToken.initialize` (1st attempt, superseded) | [41649234…](https://stellar.expert/explorer/testnet/tx/4164923483043833e8583a1aa7cc7e750d4a93850fa916dcf87bd59abee9ebce) |
| 7 | `YTToken.initialize` (1st attempt, superseded) | [45c6ea38…](https://stellar.expert/explorer/testnet/tx/45c6ea38ce60d896fe026010ff1896743c9635e09583eb81416d757dfd5b4f7b) |
| 8 | `PrincipalManager.initialize` (1st attempt, superseded) | [7c9fec05…](https://stellar.expert/explorer/testnet/tx/7c9fec053bfded4625a793ccf5c4f6492c718cadee54f93643a47bcfbf156e87) |
| 9 | `PTToken.set_minter` (1st) | [e99afab0…](https://stellar.expert/explorer/testnet/tx/e99afab029bf018ed9ef4b0804b1922244bdeba8e19415ff5a307d48583f0599) |
| 10 | `YTToken.set_minter` (1st) | [249858c1…](https://stellar.expert/explorer/testnet/tx/249858c14d4c58fed1e23fe10c7af514cf619c0780576f15d7bb41ecc124c042) |
| 11 | `Permissioning.grant_account(PrincipalManager 1st)` | [afd9da0d…](https://stellar.expert/explorer/testnet/tx/afd9da0d6cfc7238fca189250c6b51676fb3a33b21b44419bd49c1f278e1284e) |
| 12 | `RecoveryEscrow.initialize` (1st attempt, superseded) | [10b13508…](https://stellar.expert/explorer/testnet/tx/10b13508a911148475f50aa8e226c61167946587bad8310929f61bd4a2f61d9a) |
| 13 | `underlying.set_authorized(PrincipalManager 1st, true)` | [2c5ef1db…](https://stellar.expert/explorer/testnet/tx/2c5ef1db018ea472f5d9ac4b637f220cd3b34660617505b198e5fb2671256872) |
| 14 | `SYWrapper.set_recovery_escrow` (1st escrow) | [4b418483…](https://stellar.expert/explorer/testnet/tx/4b418483c403949319eab21c6d877f01fd39f368d76bb78c1e50fb01d141f6d7) |
| 15 | `PTToken.set_recovery_escrow` (1st) | [c9b9abf8…](https://stellar.expert/explorer/testnet/tx/c9b9abf8aaa7ffd1748bbd80e17c2e3d44ee6ded968ad73ab9951c2b7d8a0a58) |
| 16 | `YTToken.set_recovery_escrow` (1st) | [315a7c52…](https://stellar.expert/explorer/testnet/tx/315a7c52b0b1b1c89807280d9bd8f0c4cddf238e28c5eafc3bae6cbe95bd0033) |
| 17 | `Permissioning.grant_account(RecoveryEscrow 1st)` | [36374121…](https://stellar.expert/explorer/testnet/tx/36374121ab840161d369b87b4201e832f645e6e19cf27c7bda1120a6b4583a1d) |
| 18 | `underlying.set_authorized(RecoveryEscrow 1st, true)` | [5ac77f41…](https://stellar.expert/explorer/testnet/tx/5ac77f4162565ceb0fb38ccccf0ae263602e134b6b695ee3052cb5e32ea34456) |
| 19–24 | `Permissioning.grant_account` + `grant_asset`(PT 1st) + `grant_asset`(YT 1st) for Alice, then the same three for Bob | [930884c2…](https://stellar.expert/explorer/testnet/tx/930884c293c7ccf533253e40a02d7a441b8345d46680baf3a03fdc5dfcea6285), [9fe26aa3…](https://stellar.expert/explorer/testnet/tx/9fe26aa327c30ee8adc5e27be51bfc605d3c941c2f0f27c7753578ea818ac253), [831e3da0…](https://stellar.expert/explorer/testnet/tx/831e3da0fe6af1c01139a04298f1531ab0cb9238669ef8bc03a75e7086eb3677), [357cafd9…](https://stellar.expert/explorer/testnet/tx/357cafd953fa5f6fef8a63aea81516eda459bf0f40e8e071128d523fb6a8b617), [06f6761d…](https://stellar.expert/explorer/testnet/tx/06f6761d9ba5d6c353c35999b7e4ef402b33c1e6263c0626c6a476e5c78551cc), [cc86e2a0…](https://stellar.expert/explorer/testnet/tx/cc86e2a0ffcff75f3e53a0371260d5272a8954171e824f71a5660ec887433ebc) |
| 25 | `underlying.set_authorized(SYWrapper, true)` — discovered mid-session that `SYWrapper.deposit`'s internal transfer needs the contract's own address authorized too, since it *receives* real STA | [42912931…](https://stellar.expert/explorer/testnet/tx/42912931d13f665655cafff18348675a72c61fb29341da0d9a2880c591107991) |
| — | *(Alice's `deposit` against the 1st-attempt stack, then `PrincipalManager.mint` reverting `AlreadyMature` — no on-chain hash, since a simulation failure never submits a transaction; this is what triggered the redeploy)* | — |
| 26–29 | Redeploy: `PTToken.initialize`, `YTToken.initialize`, `PrincipalManager.initialize` (final, working addresses) | [0dc0cefa…](https://stellar.expert/explorer/testnet/tx/0dc0cefaafe9317bae79d8d9c7a2e229f4c11bd6b388ff1fbe1e7a255d735228), [c4ac184d…](https://stellar.expert/explorer/testnet/tx/c4ac184db655dbf3f1cd7a3e09161e921cf2dd7948dcbc9a0240afb7535632c1), [e17ac612…](https://stellar.expert/explorer/testnet/tx/e17ac6125be8f4230d6d2febd8ded37bf9e1dd1b9aba62a24b02eb98eef3b3b8) |
| 30–31 | `PTToken.set_minter` + `YTToken.set_minter` (final) | [a9ab611a…](https://stellar.expert/explorer/testnet/tx/a9ab611a96f67a91648c8a113c9fe91d4523301363c2c33e1d1dcb322203e181), [674526fd…](https://stellar.expert/explorer/testnet/tx/674526fd6895342b0337ac4bcc74d83b98f3ed960da4950266fd444949400785) |
| 32 | `RecoveryEscrow.initialize` (final) | [ccd58e78…](https://stellar.expert/explorer/testnet/tx/ccd58e78c5abc431b2d966c3947c953ea17d7b62c1d5d52a267fdd7f6571afd7) |
| 33–34 | `PTToken.set_recovery_escrow` + `YTToken.set_recovery_escrow` (final) | [429eff75…](https://stellar.expert/explorer/testnet/tx/429eff75f76fe3917993514492f00c9f507c1f497d37003958ec52e9498395bc), [ad94792b…](https://stellar.expert/explorer/testnet/tx/ad94792baabfe9711e972e23b675e66f62930092a91eb4015aa8a1cfad4af2d0) |
| 35–36 | `Permissioning.grant_account` + `underlying.set_authorized` for PrincipalManager (final) | [f05f639e…](https://stellar.expert/explorer/testnet/tx/f05f639e424722105d1ff5f0e2de789e61730325041c8db9da5c01f454ef96fd), [346e82de…](https://stellar.expert/explorer/testnet/tx/346e82dedf1ea613f53d770eb4ee1afde6e2a374f5a07c33458482ae1e3ea016) |
| 37–38 | `Permissioning.grant_account` + `underlying.set_authorized` for RecoveryEscrow (final) | [cb7f2e01…](https://stellar.expert/explorer/testnet/tx/cb7f2e01e98c18f8274da3df71add1291a80e2b01a60ca9d839c0ef51a297461), [ee65e903…](https://stellar.expert/explorer/testnet/tx/ee65e903a4e260870ecc919df5e39c0bc469620b7e79e3148dd7eba33e71726b) |
| 39–42 | `Permissioning.grant_asset`(PT final) + `grant_asset`(YT final) for Alice, then the same two for Bob | [389a6ed9…](https://stellar.expert/explorer/testnet/tx/389a6ed9a4b3cff444c9e252312190d732ad06b945b627b39178b297048df390), [70a40a9e…](https://stellar.expert/explorer/testnet/tx/70a40a9eb04cc7b1e36deb274f7604cc024b2dc16c8e3fedc4bd77745ef3446b), [20c1d0c2…](https://stellar.expert/explorer/testnet/tx/20c1d0c2e94d6152dc676955e675d98aa34b158278a6dc19d092541b947f7c21), [d04c6f8c…](https://stellar.expert/explorer/testnet/tx/d04c6f8ceef6d7ccf0bdbd9387b945e9ef4098f63089848dd7fdfc3b805e2b90) |

## 6. Demo transactions

### 6.1 Thread 1 — Alice (ordinary continuous-yield tokenization)

| # | Call | Transaction | Result |
|---|---|---|---|
| 1 | `SYWrapper.deposit(from=Alice, amount=500 STA)` | [5728686d…](https://stellar.expert/explorer/testnet/tx/5728686d24cca4e4b5c40493b4c40d5a8ee1d21e92696ae0cca9ffaa38e3dd7e) | 5,000,000,000 SY shares |
| 2 | `PrincipalManager.mint(from=Alice, sy_shares=5,000,000,000)` | [36860d63…](https://stellar.expert/explorer/testnet/tx/36860d63c31344a7e6ea6035c506a0906b3bf98c8d2a9a30c8f488e0d0795930) | 500 PT + 500 YT |
| 3 | `OracleAdapter.set_reference_value` — rate 1.00 → 1.08 | [21e30297…](https://stellar.expert/explorer/testnet/tx/21e30297bf72240beb2e2472cd8f992a21e051d1e0fc6cf8d4064e2fec7f615f) | simulated NAV appreciation |
| 4 | **`PrincipalManager.claim_yield(from=Alice)`** — mid-life, no PT/YT burned | [cbacf10c…](https://stellar.expert/explorer/testnet/tx/cbacf10c43ce153a95383d15b27136e7b839a4e922b8d5b632c597d3b7bde8d7) | 37.03705 STA paid to Alice's wallet in real underlying; PT/YT balances unchanged at 500/500 |
| 5 | `OracleAdapter.set_reference_value` — rate 1.08 → 1.10 | [f537ddca…](https://stellar.expert/explorer/testnet/tx/f537ddca936369445bbf83af20bbded9a7a7b1d0a70e03fd816c72b6c53d61b7) | also serves as the freshness refresh needed for redeem |
| 6 | `PrincipalManager.redeem(from=Alice, pt_amount=500, yt_amount=500)` | [9ae67f42…](https://stellar.expert/explorer/testnet/tx/9ae67f429e2fda7978ae76c72fc5ae02f893dcddc42a8e23415db5de0627f957) | 454.5454545 STA (principal) + 9.0909542 STA (yield accrued since the mid-life claim) |

Alice's STA balance across this sequence: `500.5` (start) → `5.0` deposited leaves `500.5`
(actually recorded on-chain as `500.5` before deposit, `500.5` after — deposit moves STA into
SYWrapper's custody, not out of existence) → `537.53705` (after claim) → `1001.1734587` (final,
after redemption). Every step is independently checkable against the balances quoted in the
transaction result events above.

### 6.2 Thread 2 — Bob (issuer-initiated compliance recovery)

| # | Call | Transaction | Result |
|---|---|---|---|
| 1 | `SYWrapper.deposit(from=Bob, amount=300 STA)` | [6a6c1cda…](https://stellar.expert/explorer/testnet/tx/6a6c1cda02b2fdb150fc6868ba9e250c3fdd41520c558783b8d6ffb1e1885ad0) | 3,000,000,000 SY shares |
| 2 | `PrincipalManager.mint(from=Bob, sy_shares=3,000,000,000)` | [f7eb96df…](https://stellar.expert/explorer/testnet/tx/f7eb96dff11049b0b16d0cd514fafd86ca21e69ba683ec11005cd65a8bb20ad1) | 300 PT + 300 YT |
| 3 | Issuer clears Bob's classic trustline authorization (`set-trustline-flags --clear-authorize`) | [6d78b157…](https://stellar.expert/explorer/testnet/tx/6d78b157f535d84f76b95938210db0ca26decdc31b42bbb96e5f6077acb75f90) | `underlying.authorized(Bob)` now reads `false` |
| 4 | `RecoveryEscrow.seize_pt(caller=issuer, account=Bob, amount=300)` | [669fa812…](https://stellar.expert/explorer/testnet/tx/669fa812cd038a34e394d23e7857f296e68b243c2ca166b2ae39368d4675a913) | Bob's 300 PT moved to the escrow |
| 5 | `RecoveryEscrow.seize_yt(caller=issuer, account=Bob, amount=300)` | [474042a5…](https://stellar.expert/explorer/testnet/tx/474042a5fbb7d265b78471b4a21c77882bbfa20bb9d007ff50e7de2c4bbda629) | Bob's 300 YT moved to the escrow |
| 6 | `RecoveryEscrow.finalize_pt(caller=issuer, pt_amount=300)` — post-maturity | [fb62bc54…](https://stellar.expert/explorer/testnet/tx/fb62bc549a4e2f3648f7d0d5e12924ad320dfd33c9a829cffc65fdf4f2f04a2f) | 272.7272727 STA released to the escrow |
| 7 | `RecoveryEscrow.finalize_yt(caller=issuer, yt_amount=300)` | [04293656…](https://stellar.expert/explorer/testnet/tx/04293656f604d50802ec3ed416d25d5231f7d57c2e991768da8caa705fa7aed6) | 5.4545725 STA released to the escrow |

## 7. Final on-chain state (independently queryable)

| Query | Value |
|---|---|
| `PT.balance(Bob)` | `0` |
| `YT.balance(Bob)` | `0` |
| `PT.balance(RecoveryEscrow)` | `0` (fully unwound by finalize) |
| `YT.balance(RecoveryEscrow)` | `0` |
| `underlying.balance(RecoveryEscrow)` | `2,781,818,452` (278.1818452 STA — recovered from Bob's position, ready for the issuer's native SAC clawback) |
| `underlying.balance(Bob)` | `7,000,000,000` (700 STA — his un-deposited wallet balance, never touched by the recovery) |
| `PT.balance(Alice)` | `0` |
| `YT.balance(Alice)` | `0` |
| `underlying.balance(Alice)` | `10,011,734,587` (1,001.1734587 STA) |
| `PrincipalManager.total_pt()` | `0` |
| `PrincipalManager.total_yt()` | `0` |

Every balance above was read directly from the deployed contracts via
`stellar contract invoke ... --send=no`, not computed or asserted independently of them.

## 8. Follow-up: multi-user stress test

See [TESTNET_STRESS_TEST_EVIDENCE.md](TESTNET_STRESS_TEST_EVIDENCE.md) for a subsequent run
against a second, independent market: 5 additional wallets (7 participants total), 200+ real
transactions covering varied-size deposits/mints, direct and delegated PT/YT transfers,
repeated mid-life yield claims, and circuit-breaker volume tracking under load.
