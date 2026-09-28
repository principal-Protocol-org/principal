# Principal Protocol

A Soroban-native yield tokenization protocol for regulated real-world assets on Stellar, with a native
compliance layer. It splits a yield-bearing RWA into a **Principal Token (PT)** that pays a fixed return at
maturity and a **Yield Token (YT)** that captures the variable yield, trades them on a time-aware yield-curve
AMM, and inherits every compliance control directly from the underlying asset — a classic or SEP-8 Stellar
asset through its SAC, or a SEP-57 (T-REX) RWA token.

Start with the [Tranche 1 deliverables](TRANCHE_1_DELIVERABLES.md) for what was built and how each piece is
proven, then the [yield math and fees guide](YIELD_MATH_AND_FEES.md) and the [API reference](API_REFERENCE.md).
The repository README (project overview, quick start, repository layout) is at
<https://github.com/principal-Protocol-org/principal>.
