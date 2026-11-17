# Mainnet Transaction Ledger — Tranche 3 (complete reconciliation)

Generated 2026-11-18 by cross-referencing the real on-chain transaction history of every
account used in this deployment (`mainnet-admin`, `mainnet-mockusdy-issuer`,
`mainnet-ustry-issuer`, `mainnet-gilts-issuer`, `mainnet-cetes-issuer`, `mainnet-xaum-issuer`,
`mainnet-test-a`, `principal-mainnet-treasury`) against every tx hash cited in
`docs/MAINNET_DEPLOYMENT_EVIDENCE.md` and the five per-asset deployment manifests.

**276 real, successful transactions** across these 8 accounts, 2026-11-16 onward
(the window immediately before and during Tranche 3's mainnet work).
**154 were already individually cited** by hash
in the narrative docs/manifests; **122 are added here** —
almost all of them real compliance-grant calls that were previously only described in
aggregate ("6 transactions, see the manifest's transactions array" / "see background task
output"), plus several funding transfers and a handful of incidental dust payments from
unrelated external accounts (noted as such below).

| # | Time (UTC) | Tx hash | Accounts | Action | In narrative docs? |
|---|---|---|---|---|---|
| 1 | 2026-11-16T14:11:27Z | `6b41f3e2626bf007e3c4be4bb70e9ec44e6bfd442959fab7b09b5d1283036b17` | admin, principal-treasury | create_account GBUVAH.. bal=15.5000000 | **new** |
| 2 | 2026-11-16T14:11:37Z | `9c61024b4d6bc0c488a56d4ec2c35cd168f02afa17cee44b04b6bd6cfd9c7d32` | admin | payment 0.0001000 XLM GBUVPL..→GDZN5W.. | **new** |
| 3 | 2026-11-16T14:11:37Z | `2f26ec9701c7ec17fb507c7c39cc951067e2419a3dffc1ab2c1edbbf3eba309a` | principal-treasury | payment 0.0001000 XLM GDZNUT..→GBUVAH.. | **new** |
| 4 | 2026-11-16T14:11:52Z | `e8d4b80751efab6567b2285630b1843e789574834129497d960876139c82c877` | principal-treasury | payment 36.5000000 XLM GAHNF7..→GBUVAH.. | **new** |
| 5 | 2026-11-16T14:12:02Z | `27e3ee8fb5ef14e38a440b10239a930c92445d9b7b2a4260fd1c481fa7103d76` | principal-treasury | payment 17.3000000 XLM GCDXIP..→GBUVAH.. | **new** |
| 6 | 2026-11-16T14:12:17Z | `ab748aa5c3f1affd0085b6357966e71351cccce7c93d70a74ecd504b024c8150` | test-a, principal-treasury | payment 11.3000000 XLM GCOMBI..→GBUVAH.. | **new** |
| 7 | 2026-11-16T14:12:32Z | `0392987f95db455151bef8ec73cd58f8d9532e22dd6ac55f9907cefeb6f7b260` | principal-treasury | payment 1.8000000 XLM GAVGI2..→GBUVAH.. | **new** |
| 8 | 2026-11-16T14:12:47Z | `7b901266914f40c0d23c50c75b182f08dedcf261bbd04fffa3323ba67da695e0` | principal-treasury | payment 1.3000000 XLM GDHF73..→GBUVAH.. | **new** |
| 9 | 2026-11-17T11:13:57Z | `6d6513dfe9db2b932b7b2374cb8f86add800ad07ed947f61a005a3c11fa0f256` | mockusdy-issuer, principal-treasury | create_account GB3SOE.. bal=5.0000000 | yes |
| 10 | 2026-11-17T11:14:07Z | `25ae4869c30282e84f5f56e8cf9ec76d0dfaeb339b2a89f220a30ff796d88163` | mockusdy-issuer | payment 0.0001000 XLM GBUVPL..→GB3SOE.. | **new** |
| 11 | 2026-11-17T11:14:07Z | `be7daad888aa1426ce00b4e5546c1023b22f8cdd7aa9a296fe5c55c4cf9101f0` | principal-treasury | payment 0.0001000 XLM GB3SU6..→GBUVAH.. | **new** |
| 12 | 2026-11-17T11:14:22Z | `56a51e645955e55006471fe897e8db8b7d7c834dc7fec3c352a9f8f502d3101b` | admin, principal-treasury | payment 5.0000000 XLM GBUVAH..→GDZN5W.. | yes |
| 13 | 2026-11-17T11:15:12Z | `a65d5bdafee7da783f72a03e30d2b6c368a3189350e60e23439e8175feacdc80` | mockusdy-issuer | set_options ['auth_revocable', 'auth_clawback_enabled'] | yes |
| 14 | 2026-11-17T11:15:47Z | `33f9393647837881e3bbda15c4bfbf7b12d02beaa20987c0bc2c73e9625a2aeb` | mockusdy-issuer | deploy_contract | yes |
| 15 | 2026-11-17T11:17:52Z | `a1ca431f93d7f81b5b08cc36fdb800f483ec1885ee64b514943c99620b019be5` | principal-treasury | change_trust MOCKUSDY | yes |
| 16 | 2026-11-17T11:18:32Z | `c69c82764dea39ca703f5fa22fdceb1e2d65e928dee5d02edab2f636c0fb2641` | test-a, principal-treasury | payment 3.0000000 XLM GBUVAH..→GCOMBI.. | yes |
| 17 | 2026-11-17T11:18:42Z | `0840bed66951cd7d2c06d3ba50db77ba7d2e3d10d31008a99508bf2a542cd1db` | principal-treasury | payment 3.0000000 XLM GBUVAH..→GAVGI2.. | yes |
| 18 | 2026-11-17T11:18:52Z | `7d8e5bb71d3b3dbc79be3175572d1747093dcd0330d18def8fd25851b4f926ba` | principal-treasury | payment 3.0000000 XLM GBUVAH..→GDHF73.. | yes |
| 19 | 2026-11-17T11:19:12Z | `0d0686a472c00d0306ed5147e67672a21bfd480ce19565e73ae7925b12be437f` | test-a | change_trust MOCKUSDY | yes |
| 20 | 2026-11-17T11:22:57Z | `6665b7f861c377ec2cb6734964d483658849a46556bdb56f2befe5ea4abb2e29` | admin, principal-treasury | payment 20.0000000 XLM GBUVAH..→GDZN5W.. | yes |
| 21 | 2026-11-17T11:24:17Z | `2dc9289490058ddddc45ce8a0746de687ca3dfe8191b0845722c7e6636e6b4fa` | mockusdy-issuer, principal-treasury | payment 20.0000000 XLM GBUVAH..→GB3SOE.. | yes |
| 22 | 2026-11-17T11:25:12Z | `a510fa66c3a6b7759a93a25c1116fa23a144d98f752c322e47c32dad9c189e68` | admin | upload_wasm (re-upload of already-installed code) | **new** |
| 23 | 2026-11-17T11:25:22Z | `e061c1f781c76f14277c92c539b8ad585ae2f5c392d0c8f99fdcef4f58003f37` | admin | deploy_contract | yes |
| 24 | 2026-11-17T11:25:32Z | `d92625d1c63ae551d716e70aa1fcf1ffb8ca454a2392a8402f571f626608cd64` | admin | initialize(CBCWU5..) | yes |
| 25 | 2026-11-17T15:49:22Z | `63de0c4798398569874a59404f171ef70e673f72ec9520120447f89c9449ed01` | principal-treasury | payment 391.6510631 XLM GDUQXQ..→GBUVAH.. | **new** |
| 26 | 2026-11-17T15:53:47Z | `95ea0ad5b760dd813056717db88463fbea89d78e6256c75031d8c582905d1585` | admin, principal-treasury | payment 230.0000000 XLM GBUVAH..→GDZN5W.. | **new** |
| 27 | 2026-11-17T15:54:17Z | `5d4abe937c700369abf75126d4593fc4362c2f8face75e9140a889d02c0fe6cc` | admin | upload_wasm (re-upload of already-installed code) | **new** |
| 28 | 2026-11-17T15:54:27Z | `7fbf805d54ef08a332dd58a620606ca82dd36d25b31070fa9f9eff11ee1c8ed5` | admin | deploy_contract | yes |
| 29 | 2026-11-17T15:54:37Z | `a8e39609fb856d5818fb178f28a56c0c95d0141f816e5216190536866452241d` | admin | initialize(CCKXG4..) | yes |
| 30 | 2026-11-17T15:54:52Z | `d5a85b4eba0c4326d5110065067605e4295af3600219c9fc68e5c6806192fe11` | admin | upload_wasm (re-upload of already-installed code) | **new** |
| 31 | 2026-11-17T15:55:02Z | `05e79d4da2dc06e7a25e0b381923e886926802ecb1a09d2ea991856a0b9e9b40` | admin | deploy_contract | yes |
| 32 | 2026-11-17T15:55:12Z | `f4274c9a83bb0c2653a7685fbae310877ad3de3fa44123c7fc0911bf700691c3` | admin | initialize(CBHCRR..) | yes |
| 33 | 2026-11-17T15:55:32Z | `48092ae35da2001aa8d277b4fecd5e254c18a4604df3398aa37783a3533df372` | admin | set_asset_limit(CBHCRR..) | yes |
| 34 | 2026-11-17T16:00:27Z | `d9cd44116ca70b67a11d1503d9f3c352c70c4926495edf23de4d8008efccfa4b` | admin | upload_wasm | yes |
| 35 | 2026-11-17T16:01:02Z | `cc24f84c48a703a68f0f16373250a1a4ddf388df2cb88494168144812f620921` | admin | deploy_contract | yes |
| 36 | 2026-11-17T16:01:17Z | `4cc9172b0a24c4f566bee43193bf98a613e6f65422e0f7139e61128d3d6614dc` | admin | initialize(CCM4QQ..) | yes |
| 37 | 2026-11-17T16:02:47Z | `67363c8020a4609f9bf4808f2dd8faabd6d0b45ea0049dc71b25f390a9469e85` | admin | upload_wasm | yes |
| 38 | 2026-11-17T16:03:07Z | `7ab8f3ca627588608ca54d70a7232ecffa0714e3c9294f09b90f284331beb7af` | admin | deploy_contract | yes |
| 39 | 2026-11-17T16:04:32Z | `c4fadb1054ba7cb26a8bbb1284ef121bf3cd1e890622a82c330bc75df3f3b765` | admin | set_sources(CCKXG4..) | yes |
| 40 | 2026-11-17T16:04:47Z | `f186c699affbc6267112a166cb37b0c4bb611433ce598ea2e2820ac148d19434` | admin | set_reference_value(CCKXG4..) | yes |
| 41 | 2026-11-17T16:05:12Z | `cfc891dea1cec242f442c9b585a9b69fa0b64842331bc4671a4023559f1e9d1d` | admin | set_feed_source(CCKXG4..) | yes |
| 42 | 2026-11-17T16:05:22Z | `ef2cf87e51f316877a2dd982c8ddcee141bd67f26f99b8d54386a6d6eca87ae7` | admin | set_feed_min_interval(CCKXG4..) | yes |
| 43 | 2026-11-17T16:05:42Z | `9a78fad9a8e48609e178d564db2ecf1bb1f40c144913b551f81e6e28e745d9b0` | admin | sync_from_feed(CCKXG4..) | yes |
| 44 | 2026-11-17T16:05:47Z | `e49b979a04f4b915970b085adf7e7f57bd5bbf01bd8a39429eeb0ef6b7568360` | admin | set_feed_min_interval(CCKXG4..) | yes |
| 45 | 2026-11-17T16:13:57Z | `217ac1a442801c8cd97e471971ee96f2a56a4c4d9c9b1ee293091c8a89102d1e` | mockusdy-issuer, principal-treasury | payment 180.0000000 XLM GBUVAH..→GB3SOE.. | **new** |
| 46 | 2026-11-17T16:14:27Z | `abe8ed52023709c790716f02688a8e3363e3a5b895e53b8faa7fd1cb43e6d174` | mockusdy-issuer | upload_wasm | yes |
| 47 | 2026-11-17T16:14:57Z | `5e90ef46a93fc20f9150c9d3c95e1129397738d07fe006ffa086f4a130c7233a` | mockusdy-issuer | upload_wasm | yes |
| 48 | 2026-11-17T16:15:52Z | `233514e0ebab444e40ce11e3845ffa4b5285cac3ec94c5fde1bee581aebb5268` | mockusdy-issuer | upload_wasm | yes |
| 49 | 2026-11-17T16:16:27Z | `22432394de18898744edc16571eedea3ad42612042b4e8786c620bea50f3e5df` | mockusdy-issuer | upload_wasm | yes |
| 50 | 2026-11-17T16:24:42Z | `c0dee868c77de06c0ea8d6783297faf63124ac06eaaff9e7eb83f4093dfedde6` | admin, mockusdy-issuer | payment 55.0000000 XLM GDZN5W..→GB3SOE.. | **new** |
| 51 | 2026-11-17T16:25:32Z | `3dcaca908ae2dceb2200679c05d2a0c0b1a3e0ad4f8fc33465d5a80eb5dd52a2` | mockusdy-issuer | upload_wasm | yes |
| 52 | 2026-11-17T19:20:27Z | `add9c10448cc1048f77ae2d0f9d10fae8b4d2c6d8b76396c69792f73a1f427d4` | principal-treasury | payment 170.0000000 XLM GAQZU7..→GBUVAH.. | **new** |
| 53 | 2026-11-17T19:25:07Z | `2b2b3783462c5bb1d4d36ae68c30c27853c93a1615ce41e6e343a94ac55f6f9e` | admin | set_reference_value(CCKXG4..) | yes |
| 54 | 2026-11-17T19:26:07Z | `804225bdb97fd30282ffed0277c1ea87eff9b9fed3578e5f73cb4b0a6e4e299d` | mockusdy-issuer, principal-treasury | payment 170.0000000 XLM GBUVAH..→GB3SOE.. | **new** |
| 55 | 2026-11-17T19:26:27Z | `773f9908fe82d1043e0e5a25929501cd04b9cc4334f4c69f8f131b86315ec9e4` | mockusdy-issuer | upload_wasm | yes |
| 56 | 2026-11-17T19:26:52Z | `84cfb2485b906f6601c3e4b827e6c49f3ab6b35837ea7683bfe9800f7070e2f0` | mockusdy-issuer | upload_wasm | yes |
| 57 | 2026-11-17T19:27:57Z | `f6300538c4967d298e102dd86d17b37c20270e347590c934a536d9f660668fd1` | mockusdy-issuer | create_tokens(CDYH7U..) | yes |
| 58 | 2026-11-17T19:28:07Z | `802e13bf669b923efeeb5ebc3b444460e6e72a912ea7d548c3b7d04991b75071` | mockusdy-issuer | create_engine(CDYH7U..) | yes |
| 59 | 2026-11-17T19:28:17Z | `d0599af33679a18ac1304ebd5c66b886bef9717b951be8a532eb1015ed1e1078` | mockusdy-issuer | create_pool_and_escrow(CDYH7U..) | yes |
| 60 | 2026-11-17T19:28:27Z | `2e1715687a5e5fcf0983f80f9785af7c29b06e44917f5a1cbd3c89bdc0e7abbc` | admin | add_consumer(CBHCRR..) | yes |
| 61 | 2026-11-17T19:28:32Z | `a1ae25fb627589ad5c636e630c35edd74009b20bbd73ffc193e74b1d91dc359a` | admin | add_consumer(CBHCRR..) | yes |
| 62 | 2026-11-17T19:28:42Z | `66739eb5acd1cfc255f7b44667a9de19e3deec92231506fc6e97d28c278d8282` | admin | register_market(CCM4QQ..) | yes |
| 63 | 2026-11-17T19:29:32Z | `0b326c4cc623d54aaebd1093d41606b59b3975ec2d87bd9fcaa7c2497fd691ba` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 64 | 2026-11-17T19:29:42Z | `93369df4ae81c23ac83609c70f9b3474676088dafa11c69a4db44113a33cb784` | admin | grant_account(CBCWU5..) | **new** |
| 65 | 2026-11-17T19:29:57Z | `b93e31556e9ff19335db2f8dde269d19df262c6322d8128c0436ca28ab5853a3` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 66 | 2026-11-17T19:30:07Z | `a6f60a76a083538eff52dc8af1ba2a92e056203f29cdf06c47c6f3728b0d3006` | admin | grant_account(CBCWU5..) | **new** |
| 67 | 2026-11-17T19:30:17Z | `5fa66004c0e55998a10780e4705014843236d656948d760bb80a966bd58d9606` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 68 | 2026-11-17T19:30:22Z | `ba8e9e687e256eee997741b31ae19255717d9aa17af7cadeccc995e3568565ec` | admin | grant_account(CBCWU5..) | **new** |
| 69 | 2026-11-17T19:30:32Z | `095ada0f9ba4f1637aa7506d6a2c367bd88428cfc6a641dc1e5368ff96b6eb49` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 70 | 2026-11-17T19:30:42Z | `e5cabb1760a3ca3d96dcc81812cec973ddd528fdc3be829ce0420007a9ef5948` | admin | grant_account(CBCWU5..) | **new** |
| 71 | 2026-11-17T19:30:52Z | `c0a924f0a45b2c043939cbc51a54833542b2ef43fddc98fef662a9b381dfede2` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 72 | 2026-11-17T19:30:57Z | `aa19e286192d7fb8385d454305f4ddf8a03cb4472428a1345f4ada54e90c8435` | admin | grant_account(CBCWU5..) | **new** |
| 73 | 2026-11-17T19:31:07Z | `98d3c5975b677b7f19aba4b6fd14219054ab44a036788e6206338fbb7cb66c74` | admin | grant_asset(CBCWU5..) | **new** |
| 74 | 2026-11-17T19:31:17Z | `2b68a34bda847ba13ba6231edc73db48a4716698e2e8127feb7a087420dc12ac` | admin | grant_asset(CBCWU5..) | **new** |
| 75 | 2026-11-17T19:31:27Z | `db5719dc1b746ac511a8e97daa6b463a999f74d012efd4dc2add72f3e157d4ef` | admin | grant_asset(CBCWU5..) | **new** |
| 76 | 2026-11-17T19:31:37Z | `cabb33ce32cdbe43eed5b8159a728a5edfd811779fa684193a808a1ceaafb692` | admin | grant_asset(CBCWU5..) | **new** |
| 77 | 2026-11-17T19:31:47Z | `b15f514c3e8f1216fa81cdfda0114986a5df043dcfaa10e551df518d8b5f8bd5` | admin | grant_asset(CBCWU5..) | **new** |
| 78 | 2026-11-17T19:31:57Z | `68b85da1beb9732ac17d3b479635319328ded1868bf560bd5064aaa63aaec279` | admin | grant_asset(CBCWU5..) | **new** |
| 79 | 2026-11-17T19:41:12Z | `9ef56e0fecfda733cfab952127b82b0c1332bd0fad8923c8a0ae7b305f50fd8b` | mockusdy-issuer | create_tokens(CDYH7U..) | yes |
| 80 | 2026-11-17T19:41:22Z | `5afc1deab6787694cca25c7880d956fcc0ec3150e595d6b8e667ae57f52b9c78` | mockusdy-issuer | create_engine(CDYH7U..) | yes |
| 81 | 2026-11-17T19:41:32Z | `698240cba79b7f6b3f31eac3956a29dbae13b91a7cf48bce0729b8e29b834524` | mockusdy-issuer | create_pool_and_escrow(CDYH7U..) | yes |
| 82 | 2026-11-17T19:41:42Z | `b7e4a1d0736facb67b5d496dbe582f11a5005c413a9637cc9467bd2afd1778a2` | admin | add_consumer(CBHCRR..) | yes |
| 83 | 2026-11-17T19:41:47Z | `430b3c8d1c7b48509b1d3956bfce380111eb001e934d995e7841f74b90c1cfdc` | admin | add_consumer(CBHCRR..) | yes |
| 84 | 2026-11-17T19:41:57Z | `715cdb37837ce0bb00bed7531cffbfa2ca3387ff32924f50cd5ecb0a674d3591` | admin | register_market(CCM4QQ..) | yes |
| 85 | 2026-11-17T19:42:27Z | `cf0f2466b8b83553f22f8a60588b11c7e1412429053325c5ddc80bcd5a54a47d` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 86 | 2026-11-17T19:42:37Z | `771b56067ee015a86313cb8a2f23fdfc54553db35a3f5814b0e9513001ff904f` | admin | grant_account(CBCWU5..) | **new** |
| 87 | 2026-11-17T19:42:47Z | `25b214d7f65afdd8186764e908893fcc384cb651bc5cb7e53707fcadebe0400e` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 88 | 2026-11-17T19:42:57Z | `70497ac5ced4d0ce382dc836ecf813c37872d4574082437a1752ce14722a11bf` | admin | grant_account(CBCWU5..) | **new** |
| 89 | 2026-11-17T19:43:22Z | `ca83f9f4d078a579c6bd0144bb85d66a79fb0c62d72dfe8e23d9812722bbb31e` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 90 | 2026-11-17T19:43:42Z | `207a7aa6c9a6d0def8d01a6a3b2cd909d61a56ca1fa7eba22f0cebbc1461603e` | admin | grant_account(CBCWU5..) | yes |
| 91 | 2026-11-17T19:43:52Z | `43c77b3b72825bdc52671f6f73da786600e85abd133cc095a656f4840487a873` | mockusdy-issuer | set_authorized(CDQYZU..) | **new** |
| 92 | 2026-11-17T19:44:02Z | `cd56246055cb8db5ff1eb07071d40adb4c20b906b7f4341f0d71ca71f8706a1c` | admin | grant_account(CBCWU5..) | yes |
| 93 | 2026-11-17T19:44:17Z | `a8f02ffda130749ac97bcdc9e53d43ba4a3061648364eeb3e518ff711bab7437` | admin | grant_asset(CBCWU5..) | yes |
| 94 | 2026-11-17T19:44:27Z | `c20c441d1c10c71491f250c76c383812390964e51d89a567ef00fcd30d2dd52d` | admin | grant_asset(CBCWU5..) | yes |
| 95 | 2026-11-17T19:44:37Z | `508b459344baa0ea61c80f122e468aca545518fb4d41bec4ff6c917608a9e6f2` | admin | grant_asset(CBCWU5..) | yes |
| 96 | 2026-11-17T19:44:47Z | `32a010f8eb1049a240a5eb40ebe99346298116f33c92234e9b8fa023e4e70d43` | admin | grant_asset(CBCWU5..) | yes |
| 97 | 2026-11-17T19:49:37Z | `358bb2753923e9413a78907df7a5fb08af1f5b2adec8f5c400a0b22f7d3369c9` | admin | grant_account(CBCWU5..) | yes |
| 98 | 2026-11-17T19:50:17Z | `a2c3bef0baae7d11b7f396f348dfd8abe9034fdb777a6abd7de2f4b9df4199d0` | mockusdy-issuer, test-a | payment 0.0001000 MOCKUSDY GB3SOE..→GCOMBI.. | **new** |
| 99 | 2026-11-17T19:50:42Z | `2add04a0d80106971726147966a9d85d561beff8b75cedffe418b3b366ba83a4` | mockusdy-issuer, test-a | payment 1000.0000000 MOCKUSDY GB3SOE..→GCOMBI.. | yes |
| 100 | 2026-11-17T19:51:22Z | `f3fcba50d1f2605cbe153ef9454f7119ed931d5ca0ecbea63c727d2adaea5726` | test-a | deposit(CA74VH..) | yes |
| 101 | 2026-11-17T19:55:12Z | `9db32a368b5c14f1862bbd8268d5c060ba177f28747361473a69bddac6e6822d` | admin | grant_asset(CBCWU5..) | yes |
| 102 | 2026-11-17T19:55:22Z | `a89a068720391c4ba3b623d91bb32a27b7650cc6364db3d05f9f2071b01159ee` | admin | grant_asset(CBCWU5..) | yes |
| 103 | 2026-11-17T19:55:37Z | `12f90a7b41736bceb318d13e4a413f92eedf34823f24900db492512a0375fee3` | test-a | mint(CB7VVJ..) | yes |
| 104 | 2026-11-17T19:56:27Z | `fc64092f20768394e142a64bbba1a4a89add3622af79a71ffff4fa1be6a237fc` | admin | grant_asset(CBCWU5..) | yes |
| 105 | 2026-11-17T19:56:37Z | `71dbd9e12b298ba3c8990abcb1faa74bf16cb5d5008a38a2ba5ccb1d8360fe48` | admin | grant_asset(CBCWU5..) | yes |
| 106 | 2026-11-17T19:56:57Z | `972727f72064f9acd6955339a84c544157bcb5ebc113c19ee3cd25a325ef0776` | test-a | deposit(CCY5P6..) | yes |
| 107 | 2026-11-17T19:57:12Z | `ddbdd13683ffd9b93509d9c223556aaa04d57c2d0870195db304580b8e2b4906` | test-a | mint(CBBKUZ..) | yes |
| 108 | 2026-11-17T19:57:57Z | `516490ef54181d4ca2b65c6ceed1e3596499efcd4d04b658249b1b8ed4b9b7a9` | test-a | add_liquidity(CAON3F..) | yes |
| 109 | 2026-11-17T19:58:12Z | `1bff923cff8eb97a2c810bc4a22e38b4c12f4f8ef69c41fd0799da62215c8afa` | test-a | swap_pt_for_sy(CAON3F..) | yes |
| 110 | 2026-11-17T20:00:52Z | `5686cbee284613a5a3be9392ee51518e8686a194640acb3df3fef58962fb6754` | test-a | claim_yield(CB7VVJ..) | yes |
| 111 | 2026-11-17T20:12:57Z | `d3b8edd47628d50c90e8a275e492e19ec86b8e088decdcf93da1f130d7aa3ac8` | admin | settle_all(CB7VVJ..) | yes |
| 112 | 2026-11-17T20:13:17Z | `b3a0f1a55fea5a959535590181250951ac0e78e44fbd26220ffe708a1b9c8e04` | test-a | redeem(CB7VVJ..) | yes |
| 113 | 2026-11-17T20:47:37Z | `f9fc5e03e80629410e93fe727df4e4b0ab2bb906abac6c8e403c7679bcc80a7d` | mockusdy-issuer, principal-treasury | payment 20.0000000 XLM GB3SOE..→GBUVAH.. | yes |
| 114 | 2026-11-17T20:48:12Z | `b22bfdc532c9ff02ac394a2ff163a47f764e99376669081790dc30a1ad298cda` | ustry-issuer, principal-treasury | create_account GDM2Q2.. bal=3.0000000 | yes |
| 115 | 2026-11-17T20:48:27Z | `eb069fe0875af4f67411fb73a197531c73cd6361fe68cec981d2ae6d64f19d37` | ustry-issuer | payment 0.0001000 XLM GBUVPL..→GDM2Q2.. | **new** |
| 116 | 2026-11-17T20:48:27Z | `b931aa5138f9aabbc049a56e8a96823c43c018ce0fbe67dc031c7171dc528a31` | principal-treasury | payment 0.0001000 XLM GDM2FS..→GBUVAH.. | **new** |
| 117 | 2026-11-17T20:48:52Z | `201b32a7aada1a2b9e8c084de5ea458c6970558b95840534e0e4a25bc4e8f025` | ustry-issuer | set_options ['auth_revocable', 'auth_clawback_enabled'] | yes |
| 118 | 2026-11-17T20:49:27Z | `07b4a96d80ef4b81423a8277ec36307211bc70aa44b1c861cc9342252f3c115d` | ustry-issuer | deploy_contract | yes |
| 119 | 2026-11-17T20:52:12Z | `1e68d0cb7d808f7fc6c5f9cf168ffa3fce37155f2f6c51118c88f68dda347562` | admin | deploy_contract | yes |
| 120 | 2026-11-17T20:52:22Z | `c02269cd1617ce8d866c8c91c6b495b3768bab1c0f3bb5e3a41cc4c453cf1dbd` | admin | initialize(CBHAWS..) | yes |
| 121 | 2026-11-17T20:52:32Z | `946d7e343719eb7c81aa1f36f82fb3e7402931324ea47a269b6f92bca06d2c14` | admin | set_asset_limit(CBHCRR..) | yes |
| 122 | 2026-11-17T20:52:47Z | `a509378dcb09d2186f2309a553e55bdc93f9abe3ce6fdb9f6b56525f98574097` | admin | set_sources(CBHAWS..) | yes |
| 123 | 2026-11-17T20:52:57Z | `b6e45c17816264ea2ce0c2f1bdde7a9ace3f185e8c23e1f5df258467b29028ee` | admin | set_reference_value(CBHAWS..) | yes |
| 124 | 2026-11-17T20:53:07Z | `632775894ee68bdfc731f5a0339ce5761e3d6ab3684849c1c94a1c812d66c719` | admin | set_feed_source(CBHAWS..) | yes |
| 125 | 2026-11-17T20:53:17Z | `3bde78b1f815d674448753ecc5d67e0a8ef4a3581a0f9878b1ec70bc7805fadd` | admin | set_feed_min_interval(CBHAWS..) | yes |
| 126 | 2026-11-17T20:53:27Z | `28daacca78b993cf5a48fe10b35080e072a359fe9d4db28aeae884d67c8e2101` | admin | sync_from_feed(CBHAWS..) | **new** |
| 127 | 2026-11-17T20:53:37Z | `dc5d6336d6a48408ad4c23d52cc6b45f135a817c0d9dcaef0eddf3354da20933` | admin | set_feed_min_interval(CBHAWS..) | yes |
| 128 | 2026-11-17T20:53:57Z | `eaeecc87878b8d289e8036e9eacffc234d83e5bc5391184988ec2e09621ecadb` | ustry-issuer | create_tokens(CDYH7U..) | yes |
| 129 | 2026-11-17T20:55:47Z | `473699166de01be3e40162d28beeaa9761048eb990fde3f640da585df919c2b0` | admin | set_feed_source(CBHAWS..) | yes |
| 130 | 2026-11-17T20:55:57Z | `14fd340b87a3a2fb5c3e6ec41457f90bf88df6535ae4d24618e88ff402a7d8fa` | admin | set_feed_min_interval(CBHAWS..) | yes |
| 131 | 2026-11-17T21:11:17Z | `5a6e3fad5d8d278996cf0ca060c922b51c781c1328a1f8cb4286aeb7514c711d` | admin | set_feed_source(CBHAWS..) | yes |
| 132 | 2026-11-17T21:11:27Z | `a4a0f68eeea94c1f0827526981f6018568f9165ae0acfa151fb4ab23c0fc34e0` | admin | set_feed_min_interval(CBHAWS..) | yes |
| 133 | 2026-11-17T21:22:47Z | `c77e3f1567388ac98882d69bcade313671d7a8155c3fbf8f1ade3afee80e1b09` | ustry-issuer | create_engine(CDYH7U..) | yes |
| 134 | 2026-11-17T21:23:17Z | `8f02b4f8a60d3efe9296f6b86b6ce4082b07ff6a8fc09db4e53883bd18ed0535` | ustry-issuer | create_pool_and_escrow(CDYH7U..) | yes |
| 135 | 2026-11-17T21:23:57Z | `91147c2300668f2a95ed04a2c8eea1ae97e67af791253fa3d5182572bddc8055` | admin | add_consumer(CBHCRR..) | yes |
| 136 | 2026-11-17T21:24:12Z | `cfc616c52a9513c4187443d7211c549a11c4af4a59da2fbc1680d0fc024eaf2c` | admin | add_consumer(CBHCRR..) | yes |
| 137 | 2026-11-17T21:24:22Z | `f04cc55868a2f54fce60d66d2172a3d5a5f2499a40fe286f5b6b2806fe5bab90` | admin | register_market(CCM4QQ..) | yes |
| 138 | 2026-11-17T21:24:57Z | `fd23e59e8e14f0c783ae8151f6259c478be2abf5f17c16d1ce606b774ece0069` | ustry-issuer | set_authorized(CDHQ4B..) | **new** |
| 139 | 2026-11-17T21:25:07Z | `00cf5a892e7786eed497d06db02e8ec1c8fd69fffc8acf90ddf5be41bd27a3d0` | admin | grant_account(CBCWU5..) | **new** |
| 140 | 2026-11-17T21:25:17Z | `15e55b432cd5e270f84fce88f8c8eb607f59882e1d1971c9ccbc01e98db70e3c` | ustry-issuer | set_authorized(CDHQ4B..) | **new** |
| 141 | 2026-11-17T21:25:27Z | `0b9fa468248294b59b8ef3de0adb7ccee25c6230b221474b1abb78b3732fabbf` | admin | grant_account(CBCWU5..) | **new** |
| 142 | 2026-11-17T21:25:42Z | `efb48a31e84abfc7285f6fae0a683997d11bc726ccef0d61c05e2ec8a3926204` | ustry-issuer | set_authorized(CDHQ4B..) | **new** |
| 143 | 2026-11-17T21:25:57Z | `824fd5823484c13a376fa874407318822d59b12f2c3c3bd60459e85b776f9eba` | admin | grant_account(CBCWU5..) | **new** |
| 144 | 2026-11-17T21:26:12Z | `e3032e5e9bb7883015fe156d993a898e4133e0d62dcf9ed85659558d1d5b222a` | ustry-issuer | set_authorized(CDHQ4B..) | **new** |
| 145 | 2026-11-17T21:26:32Z | `740b14717427db94d8889b00eafef6adc80f7e5c356050183b17eaca0c84a783` | admin | grant_account(CBCWU5..) | **new** |
| 146 | 2026-11-17T21:26:52Z | `abe9cefe7345b4fc708807236257e371a9e12d200862b0e58b1ee28b3a623495` | admin | grant_account(CBCWU5..) | **new** |
| 147 | 2026-11-17T21:27:02Z | `a34a0c8dde32da055f681520d1829ed9e748c07293ca852449b0ec84ede49c5c` | admin | grant_asset(CBCWU5..) | **new** |
| 148 | 2026-11-17T21:27:12Z | `6c1d53842153137937e15ec0906d7b75b091005c4cda63b58edfc02ff284ed8c` | admin | grant_asset(CBCWU5..) | **new** |
| 149 | 2026-11-17T21:27:22Z | `28dd6588f11b98d2e885d18d7994899a9df290db51479acf2feea6bbee0fcfdb` | admin | grant_asset(CBCWU5..) | **new** |
| 150 | 2026-11-17T21:27:27Z | `a2ba0baa58407a6bcd3ede0f7422fed5f660fff52e9d0c6c900a97a3ae21e49c` | admin | grant_asset(CBCWU5..) | **new** |
| 151 | 2026-11-17T21:27:37Z | `ae941ac0589cb2878473c999e0dc99af4dea3a80358171115e9b349a2d9361ab` | admin | grant_asset(CBCWU5..) | **new** |
| 152 | 2026-11-17T21:27:52Z | `b561eedeac56697553bb39eaed7f2afc70b79b7317156b469d564fb4c59e4b4e` | admin | grant_asset(CBCWU5..) | **new** |
| 153 | 2026-11-17T21:28:02Z | `218928478eb5af788b4e0003989ef26c35b0d540fde7b2335899c37adca6a7cc` | admin | grant_asset(CBCWU5..) | **new** |
| 154 | 2026-11-17T21:28:12Z | `a9eec69306800aa4f68e97916c61d41c84ddf8ce741fa2b4c1b854ec0db7f4fe` | admin | grant_asset(CBCWU5..) | **new** |
| 155 | 2026-11-17T21:34:42Z | `8981e8a4976262855aed9c98666622dfe900b00e2016b03efa038f1937b75545` | test-a | change_trust MOCKUSTRY | yes |
| 156 | 2026-11-17T21:35:17Z | `90c24dffed12b1edba1e289fb76043dd14dcb726c77468d28d2b535437ccaa05` | ustry-issuer | set_authorized(CDHQ4B..) | yes |
| 157 | 2026-11-17T21:35:32Z | `fb9b93f3ec98795196ea7dcffeea792852125e24288ee4c1ec81e0c5f1e94f41` | test-a, ustry-issuer | payment 1000.0000000 MOCKUSTRY GDM2Q2..→GCOMBI.. | yes |
| 158 | 2026-11-17T21:37:12Z | `bef5117ea188a5389642b4ed78cac0b4cec412d7bbf3a27269ce22212427e9fd` | test-a | deposit(CA4EMP..) | yes |
| 159 | 2026-11-17T21:38:32Z | `d3691d05aa26e440fdbde785a1d68811c8eef80af53d37374908ab668c489298` | test-a | mint(CAB6LH..) | yes |
| 160 | 2026-11-17T21:42:12Z | `6c34224e2d22892fcc44bc7dd1953a613cff9e418334823aa00cbcd0730cf1d7` | gilts-issuer, principal-treasury | create_account GA5KXE.. bal=3.0000000 | yes |
| 161 | 2026-11-17T21:42:22Z | `23c767c1ffd894a55940bb2a46929a43ccbdd27a120726b3dbcd9f427b5ad825` | gilts-issuer | payment 0.0001000 XLM GBUVPL..→GA5KXE.. | **new** |
| 162 | 2026-11-17T21:42:22Z | `173cfbe781459c3a3ae49fd8bbdeb32b89924518a7835eddc49bc6d2074c567c` | principal-treasury | payment 0.0001000 XLM GA5KL7..→GBUVAH.. | **new** |
| 163 | 2026-11-17T21:42:32Z | `b7ede1c47a17e53178c68f0049fbe244ced2d681ce44a6ff00279f4c3dcccd79` | gilts-issuer | set_options ['auth_revocable', 'auth_clawback_enabled'] | yes |
| 164 | 2026-11-17T21:43:02Z | `ca114f96670387e3741016db4a451dbb6d2dde4dbb5479ebebee2614b9d6fe6b` | gilts-issuer | deploy_contract | yes |
| 165 | 2026-11-17T21:43:22Z | `60143de045ced4f69741ad4ca4133d6aa387c6c677aba5383f404e73deb274ff` | admin | deploy_contract | yes |
| 166 | 2026-11-17T21:43:42Z | `0a3febe495a1e2656e097a8a498e04ba05780302090f71342fe72706c15f26bd` | admin | initialize(CDZS34..) | yes |
| 167 | 2026-11-17T21:44:02Z | `f3e748aaf2b88b615c30793343184e8e087a7e21a5e42e83acb1784fbf6bee6c` | admin | set_sources(CDZS34..) | yes |
| 168 | 2026-11-17T21:44:12Z | `ed8b732c22702b8772ee270da7e4e20e9fb775e322d79329cf9287b2c5c3c421` | admin | set_reference_value(CDZS34..) | yes |
| 169 | 2026-11-17T21:44:57Z | `5b4e44362c4e8a57fd9f088a5799015a65fe926eaf5bcd8111139f841ab48ced` | gilts-issuer | create_tokens(CDYH7U..) | yes |
| 170 | 2026-11-17T21:45:37Z | `876e6b39a9e45afb6f6e828f31b725667e81236a5f3107eaf4e6e74ac4ac52d2` | gilts-issuer | create_engine(CDYH7U..) | yes |
| 171 | 2026-11-17T21:46:07Z | `f4b70aa5a5143b4cea46080d9db8b7a8b1364376fcfb26198ce00adbdcedb1d7` | gilts-issuer | create_pool_and_escrow(CDYH7U..) | yes |
| 172 | 2026-11-17T21:46:32Z | `32d0f018fb32a8e93deefc73c4cdeedfc5fb2f82cda488f9bc3a1e1e86467107` | admin | add_consumer(CBHCRR..) | yes |
| 173 | 2026-11-17T21:46:42Z | `5f927ebdc77cd4f029eb9b5e551c09253ed8b5e3ad4287e5ebdc00b06fb6af85` | admin | add_consumer(CBHCRR..) | yes |
| 174 | 2026-11-17T21:46:57Z | `14de3a4a963f42bdfeed171e072249733385c3db1399cd7b07f96895049d6f9f` | admin | register_market(CCM4QQ..) | yes |
| 175 | 2026-11-17T21:47:17Z | `e3758479c2ac652c12f7615625ae435a098483a842eaadeddc0b99002979171d` | test-a | change_trust MOCKGILTS | yes |
| 176 | 2026-11-17T21:48:52Z | `d1e377ec9b306a70e903256ad8ac4b8c3919eeddf9ceb0a56c25ab2db75f637f` | admin | grant_account(CBCWU5..) | **new** |
| 177 | 2026-11-17T21:49:02Z | `4167ec9c8c2c01ca7017dc0e7b604d4086ccc277c62313b11d5fa076c60315c3` | gilts-issuer | set_authorized(CBBF4H..) | **new** |
| 178 | 2026-11-17T21:49:12Z | `01205b090bb5b053fbcf01fe9c0edc41a28e21ef4e8e4887a4148944b8e136f7` | admin | grant_account(CBCWU5..) | **new** |
| 179 | 2026-11-17T21:49:22Z | `295da5b431d3ece599d65f69484c0be160418242c742f9e21c46d9e16d481cee` | gilts-issuer | set_authorized(CBBF4H..) | **new** |
| 180 | 2026-11-17T21:49:32Z | `ae4ef4661dfd1d40f5483f57f857d5cb1c99298a5fb81cda79b3d106594cf99d` | admin | grant_account(CBCWU5..) | **new** |
| 181 | 2026-11-17T21:49:42Z | `4a8a7aba9a211eef3ac5f8f5e49ddc8efc13ffeeeb4a56f133f76c3dd58bfe98` | gilts-issuer | set_authorized(CBBF4H..) | **new** |
| 182 | 2026-11-17T21:49:52Z | `d2357fd70490c457351110fb450ae1cb54fde5714096f08897809447f1bcdd5f` | admin | grant_account(CBCWU5..) | **new** |
| 183 | 2026-11-17T21:50:02Z | `502cf64eb6c5a0d77db535d08d4d2b5970b8185fdac7db79933e643e21d9cbd9` | admin | grant_account(CBCWU5..) | **new** |
| 184 | 2026-11-17T21:50:17Z | `474ba1dd3ac3649bc25da745bb627a7f03878c30053a4308733f46729c0f6e1c` | gilts-issuer | set_authorized(CBBF4H..) | **new** |
| 185 | 2026-11-17T21:50:27Z | `ece864a0e98c3b1385fc6c5995154db48ff4bda6afd4fa587fbbe0dabf418dc3` | admin | grant_account(CBCWU5..) | **new** |
| 186 | 2026-11-17T21:50:37Z | `471b180841a6743dedbbea895ae88b51ae1f962b99107ba2f2014f5ae3cbd7fa` | admin | grant_asset(CBCWU5..) | **new** |
| 187 | 2026-11-17T21:50:47Z | `eea411cd1803b7e5f13e6864e86a9e81a61c2236295493245cf0eca8009df0c6` | admin | grant_asset(CBCWU5..) | **new** |
| 188 | 2026-11-17T21:50:57Z | `122ae3f2d386740db552bf1a28f140ded17b5d56d5c5a65c52db358ea700f951` | admin | grant_asset(CBCWU5..) | **new** |
| 189 | 2026-11-17T21:51:07Z | `b98424c9432a5bc88b7cccda414bb4a09a7944a9c78919504d188f130ecba327` | admin | grant_asset(CBCWU5..) | **new** |
| 190 | 2026-11-17T21:51:22Z | `206845f051f382a2abf1b8a05ee2f327b4f7ed562dfd97b14517293a4c33732a` | admin | grant_asset(CBCWU5..) | **new** |
| 191 | 2026-11-17T21:51:32Z | `64a25b75331d732c0d2a7c42c7ba781c0395cfb5b8fe7825ad9b2e78b8af9068` | admin | grant_asset(CBCWU5..) | **new** |
| 192 | 2026-11-17T21:51:42Z | `463dd4397c4d0bdf4d418c4e8a983c747b1e0199cde7ce669d85d7a2ee0bf1ed` | admin | grant_asset(CBCWU5..) | **new** |
| 193 | 2026-11-17T21:51:52Z | `0709a49e35cf68dd4b09c149d5a2ccf29f59472ad7a5af9d777021d8989c4596` | admin | grant_asset(CBCWU5..) | **new** |
| 194 | 2026-11-17T21:55:02Z | `ef6e4ed5f9f5e7bd9f6f66c586285c3ae6296aa63ac11cd9fdb22868de8c37b6` | gilts-issuer, test-a | payment 1000.0000000 MOCKGILTS GA5KXE..→GCOMBI.. | yes |
| 195 | 2026-11-17T21:55:27Z | `ce0a3dda55836e5fa84f5c30ecbd0f4539e1bc2af7d3d655ecee57e156d63927` | test-a | deposit(CDXXRJ..) | yes |
| 196 | 2026-11-17T21:55:37Z | `3536b1d5483e71bacdd72534e1cb8b709ea558e2f000cafffac60abd6aab0670` | test-a | mint(CBOJIJ..) | yes |
| 197 | 2026-11-17T21:56:07Z | `d11e29aba04c6b67d9a4bf706effd21d1c3cbf28d3601ed9295f8c2b19fd5c09` | admin, mockusdy-issuer | payment 5.0000000 XLM GB3SOE..→GDZN5W.. | yes |
| 198 | 2026-11-17T21:58:12Z | `146832cad201a8a250b15d1245a7769ceb09160960a281bf8cfe45eaca9e0e14` | cetes-issuer, principal-treasury | create_account GBB6TI.. bal=3.0000000 | yes |
| 199 | 2026-11-17T21:58:22Z | `4441da225e54d47d6ecc3dfe3a2a01366c09d7a8886d40980086f617a1d995e0` | cetes-issuer | payment 0.0001000 XLM GBUVPL..→GBB6TI.. | **new** |
| 200 | 2026-11-17T21:58:22Z | `659d998f99353876e795e6ade443a7544a5a8679643068dcd794b4b0bd1fd455` | principal-treasury | payment 0.0001000 XLM GBB6L5..→GBUVAH.. | **new** |
| 201 | 2026-11-17T21:58:32Z | `10f82a6c8bf3b3b002bfc25bdd21e412f1dd06ee9fed4648cd9574cd66d0fa4b` | cetes-issuer | set_options ['auth_revocable', 'auth_clawback_enabled'] | yes |
| 202 | 2026-11-17T21:58:57Z | `58d6f13747133e5371cde8248525f0db4f6a955c5cf1971509c2dfbdefd995a8` | cetes-issuer | deploy_contract | yes |
| 203 | 2026-11-17T21:59:22Z | `58273814dfe9e3efbdc49f493a8190ce68641ac04f341175699f1385d3678e9f` | admin | deploy_contract | yes |
| 204 | 2026-11-17T21:59:37Z | `c179f69b9e8019c779b703b8a5ea8959368cadd51355f642c2ca9fe48c2019af` | admin | initialize(CBJK32..) | yes |
| 205 | 2026-11-17T22:00:02Z | `4a16b36395576955bca2613e73ca1577b0c1dadc25b0c1b16ad63c205f039eb3` | admin | set_sources(CBJK32..) | yes |
| 206 | 2026-11-17T22:00:12Z | `d58d1321e00b415b6ad8020a32c1caec3b937b7af4cd413f68c6a06552213f2d` | admin | set_reference_value(CBJK32..) | yes |
| 207 | 2026-11-17T22:00:42Z | `5c75aef144d7c9fa5d8a5c7f8d3ae44fb0bd2d539939da100e9f3ad53c8559dc` | cetes-issuer | create_tokens(CDYH7U..) | yes |
| 208 | 2026-11-17T22:01:17Z | `ecc70bfd4a4d26c6cbec13c10e6d92ca1fa762ccedf3d65304767f9fe596eb44` | cetes-issuer | create_engine(CDYH7U..) | yes |
| 209 | 2026-11-17T22:01:47Z | `a1b8dfdc0bdba9eda44b5418945c084547c4ebba3e4a67dd3716793cca306329` | cetes-issuer | create_pool_and_escrow(CDYH7U..) | yes |
| 210 | 2026-11-17T22:02:07Z | `6959a626a6ca63210118e22f75a658820273db5853238e5213805f86b301b9dd` | admin | add_consumer(CBHCRR..) | yes |
| 211 | 2026-11-17T22:02:17Z | `48d5ddeea0d1b7168e1ee4d2b38091a58651030b63ba3ea581eec8e8146406ab` | admin | add_consumer(CBHCRR..) | yes |
| 212 | 2026-11-17T22:02:27Z | `c39581d79b602e3847a9a62f6a1c5d24218672452d68fe0fe13d3e733b7614d7` | admin | register_market(CCM4QQ..) | yes |
| 213 | 2026-11-17T22:02:42Z | `1775f1671458f62214ec0ccd130e2e649306520400c06556b0bef127ca733b46` | test-a | change_trust MOCKCETES | yes |
| 214 | 2026-11-17T22:03:27Z | `bcdce4f143e211f061b5cb19f0bb8d6d1ce881d5dc6e2b5b6a743c3286bca253` | cetes-issuer | set_authorized(CBVSL3..) | **new** |
| 215 | 2026-11-17T22:03:32Z | `8d2e39f07207e91adc24a9b185a6c55ba4b37a2bca500a54330de4d1a47caa1e` | admin | grant_account(CBCWU5..) | **new** |
| 216 | 2026-11-17T22:03:42Z | `44756720cc7bdeabc13734baa2ff2fb0ec5076db0f40b26206179d12b272a685` | cetes-issuer | set_authorized(CBVSL3..) | **new** |
| 217 | 2026-11-17T22:04:37Z | `dc747b8242a404bcd54598f1a6ec4dfe3e1c5b429da98946ed9a6a1bbe7460ae` | cetes-issuer | set_authorized(CBVSL3..) | **new** |
| 218 | 2026-11-17T22:04:42Z | `a18beea2f5ed5eef9cf9e96b975086a34d690da138a7e7332cadd90b439c85da` | admin | grant_account(CBCWU5..) | **new** |
| 219 | 2026-11-17T22:04:52Z | `d7c4108c8abaa9b2269710d721d8e647daf3317423bf0be5b0e82afed2c2af6c` | cetes-issuer | set_authorized(CBVSL3..) | **new** |
| 220 | 2026-11-17T22:05:07Z | `b81c855acbf354e07e36273f86e95c6e60bcf4ad117a137b22aa797430b871c1` | admin | grant_account(CBCWU5..) | **new** |
| 221 | 2026-11-17T22:05:22Z | `4bcd7a034f645f566b990c3a6090bcd1ff5f9297e94ce2ed1ecfe1edfa2793e4` | admin | grant_account(CBCWU5..) | **new** |
| 222 | 2026-11-17T22:05:32Z | `df24519e17c63c0c4bf0ef6a0f3fb37b3c2740bac265d66819f17af14f097896` | cetes-issuer | set_authorized(CBVSL3..) | **new** |
| 223 | 2026-11-17T22:05:42Z | `3880035c845f090f7604c453761317cd6e47fb568a80c20956cdb1a8f30b59eb` | admin | grant_account(CBCWU5..) | **new** |
| 224 | 2026-11-17T22:05:52Z | `8017a039df8e77a2e030a28215de67f4e08327ab646ed7bc1b4f616141e74216` | admin | grant_asset(CBCWU5..) | **new** |
| 225 | 2026-11-17T22:06:02Z | `ec34389bb77bff63276919dbe28292133b2212eeae9a14dd17d2a7a15ed0a460` | admin | grant_asset(CBCWU5..) | **new** |
| 226 | 2026-11-17T22:06:17Z | `2997efc77060d2a1edc037253363bfbaeeca402acaf82cd04b27cadecb915dc7` | admin | grant_asset(CBCWU5..) | **new** |
| 227 | 2026-11-17T22:06:27Z | `56709f2f136fdea2814e0381528b0fe049a78fb7879085e95077ba833a1ccd39` | admin | grant_asset(CBCWU5..) | **new** |
| 228 | 2026-11-17T22:06:37Z | `b573caedb64499069f6c78af612499968c2920de1b2fd0b56346e1d3a9c049ec` | admin | grant_asset(CBCWU5..) | **new** |
| 229 | 2026-11-17T22:06:47Z | `6170d367d93b00b598f555f12b3ff927554b7070f192876ce60870a71d82938e` | admin | grant_asset(CBCWU5..) | **new** |
| 230 | 2026-11-17T22:06:57Z | `8c417ad0680982fbc5c854fe6d9049ec66fdd97582f1ff57b0f076806dd1a2c8` | admin | grant_asset(CBCWU5..) | **new** |
| 231 | 2026-11-17T22:07:07Z | `989091af5c3d761ded69c1b6f32213feea7468796e3c4535379cc92b9a2379cc` | admin | grant_asset(CBCWU5..) | **new** |
| 232 | 2026-11-17T22:10:42Z | `cfec0e95a4e9bc4d35d8ca79abaeb23ce39b43815d8008fbda72c7129e8280da` | admin | grant_account(CBCWU5..) | yes |
| 233 | 2026-11-17T22:12:42Z | `e36f43cff1e2ba4d3a2187ec05d95af7f35e4e0ec5addbea6ef21a8d9c86cd10` | cetes-issuer, test-a | payment 1000.0000000 MOCKCETES GBB6TI..→GCOMBI.. | yes |
| 234 | 2026-11-17T22:13:12Z | `172ad551eaa0e61f5c11d09cad8b423a3adbeee15f9a78eb271d2b94943861e9` | test-a | deposit(CCSETN..) | yes |
| 235 | 2026-11-17T22:13:17Z | `20ad9b9a233c71c80339050091a47cff1bd87dd7458cedb0e4f8e6a4fb271fcb` | test-a | mint(CDOZH7..) | yes |
| 236 | 2026-11-17T22:15:17Z | `2a430491f68e0bfdf06f0f3a0aa84c50818d0e53fabda18bdac19550fa8b6798` | xaum-issuer, principal-treasury | create_account GC6VP3.. bal=3.0000000 | yes |
| 237 | 2026-11-17T22:15:27Z | `5e43687dd29fb91e852e7a3f18d716762445db6b39924819bc25c3b2f9f6bbaa` | xaum-issuer | payment 0.0001000 XLM GBUVPL..→GC6VP3.. | **new** |
| 238 | 2026-11-17T22:15:27Z | `0445306a9a6ab80ad20587cf16b37c4c20b90effdf0bd31d5b87a88f684a7f25` | principal-treasury | payment 0.0001000 XLM GC6VLY..→GBUVAH.. | **new** |
| 239 | 2026-11-17T22:15:37Z | `504cd38eb506d9ad644bcd46134f4bed4c5541037904eb03b01dbbf8fdb3463e` | xaum-issuer | set_options ['auth_revocable', 'auth_clawback_enabled'] | yes |
| 240 | 2026-11-17T22:16:07Z | `762968e7c4a8ceaba2f94956717628de2d63146a42fea506071d7c3ee0bd3f35` | xaum-issuer | deploy_contract | yes |
| 241 | 2026-11-17T22:16:22Z | `1d04f8bad3d758d6c245d79a0b35a5d9a3aa57e6523ed627d717d751396083dc` | admin | deploy_contract | yes |
| 242 | 2026-11-17T22:16:42Z | `5aca494499a63918693ee59228765e6d7cb0d62462764333782b0f8493111415` | admin | initialize(CCLB6R..) | yes |
| 243 | 2026-11-17T22:17:07Z | `653176becab77500e3bd9b0637338f228c202370deb53d8fe67ced4ecf21393b` | admin | set_sources(CCLB6R..) | yes |
| 244 | 2026-11-17T22:17:22Z | `a7ace84d06c66659ab64c32911449fcc3edaad574ad7cf305b2f88a3f75bb36c` | admin | set_reference_value(CCLB6R..) | yes |
| 245 | 2026-11-17T22:17:52Z | `744ec7d354ef9cb1afa549254a7e8ac59aca28a0b1f47f6733f7d69f70f10fcd` | xaum-issuer | create_tokens(CDYH7U..) | yes |
| 246 | 2026-11-17T22:18:27Z | `110edfad059b7175ed898e7f14c7d090b2a40beb4bef456e491b2c0576594ad8` | xaum-issuer | create_engine(CDYH7U..) | yes |
| 247 | 2026-11-17T22:18:57Z | `7dc5903bbd24607cab3a3a93e549d609704c38b9fb2606944fc58dbb56c3691d` | xaum-issuer | create_pool_and_escrow(CDYH7U..) | yes |
| 248 | 2026-11-17T22:19:22Z | `8e57ed778b5ff5f9e74ba5c67b68e245b870d8c3d0dea16356d0301919a45ce9` | admin | add_consumer(CBHCRR..) | yes |
| 249 | 2026-11-17T22:19:32Z | `3c76375e62728353aeca1e139660ed592d196d70ed4e6ecda199e64aeb870d6d` | admin | add_consumer(CBHCRR..) | yes |
| 250 | 2026-11-17T22:19:42Z | `bab156e2506c898325937e08784821a1b46caa38b9b6721bfcf1e255bd56d8c7` | admin | register_market(CCM4QQ..) | yes |
| 251 | 2026-11-17T22:20:37Z | `ba0590464dcbe9d3f8fc02e43752672eb2fa33c0f0e069d4a1fe7c6412f98dc8` | mockusdy-issuer, test-a | payment 2.0000000 XLM GB3SOE..→GCOMBI.. | yes |
| 252 | 2026-11-17T22:21:02Z | `c25c74e5f8c8baacea9b34dea59d729aaae740fcd193b9f0a318d023866acd96` | test-a | change_trust MOCKXAUM | yes |
| 253 | 2026-11-17T22:21:42Z | `befe4367cdcae9218405761b5083593e6e9b4ca8a4f42b86be7a2f18dd1b6ece` | xaum-issuer | set_authorized(CBAJVV..) | **new** |
| 254 | 2026-11-17T22:21:52Z | `c2f39e4ba08dfa6d302899e00f3b2dd4bc52eeefda197ab2aa49d7d4c6f0ca6e` | admin | grant_account(CBCWU5..) | **new** |
| 255 | 2026-11-17T22:22:02Z | `da30e2399c722ef63b397ee084a4459a7bea75f1daa0e5ae5c54467f25cea22d` | xaum-issuer | set_authorized(CBAJVV..) | **new** |
| 256 | 2026-11-17T22:22:12Z | `d33ddba2bc04685c080025c6f5709d76e946bcf20c33eaea7e6cae618b0f4a9e` | admin | grant_account(CBCWU5..) | **new** |
| 257 | 2026-11-17T22:22:22Z | `b2a16db8dd9e14730dd5fdfc22d92eb45093d717506af6f6127033472afbefcc` | xaum-issuer | set_authorized(CBAJVV..) | **new** |
| 258 | 2026-11-17T22:22:32Z | `6a3c27198809962b472589102063da9f0ddc0787c5f4e2acb5581f361ce32bb1` | admin | grant_account(CBCWU5..) | **new** |
| 259 | 2026-11-17T22:22:37Z | `dc141ee9896edd8fe38fe4638b76c15d2f9baf15ffd3d25d8df74545fde2b136` | xaum-issuer | set_authorized(CBAJVV..) | **new** |
| 260 | 2026-11-17T22:22:47Z | `0e346e93563c1bacc2b0a7111b04687c4463cd886a5c172ea97338c35c9ac39a` | admin | grant_account(CBCWU5..) | **new** |
| 261 | 2026-11-17T22:23:52Z | `0227bf2902eed9bb28333dba9dab6c22bcee385b36c64d263ad6175b7903f815` | xaum-issuer | set_authorized(CBAJVV..) | **new** |
| 262 | 2026-11-17T22:24:02Z | `90b2bf8b0d697621e5adc3910184307126da1340fa9f72f965961e1e39227f90` | admin | grant_account(CBCWU5..) | **new** |
| 263 | 2026-11-17T22:24:12Z | `de49b03539a020dde58039ae354da38aa0e924e1847be139d5baa425901a7fe8` | admin | grant_asset(CBCWU5..) | **new** |
| 264 | 2026-11-17T22:24:22Z | `a28964f573e7d60cadffb2fcbe9b74246b11e2f5992e2a644c988ea55617be7b` | admin | grant_asset(CBCWU5..) | **new** |
| 265 | 2026-11-17T22:24:32Z | `e5a1bbccefedfbf2a0d02319e4a172b498ea6918428b6a254ad38b317d3193e3` | admin | grant_asset(CBCWU5..) | **new** |
| 266 | 2026-11-17T22:24:42Z | `a5c96f605f6031fc455f459436a12f45d879667d2e62d063c6917f7200889aa0` | admin | grant_asset(CBCWU5..) | **new** |
| 267 | 2026-11-17T22:24:52Z | `c0fb3a73506bfe4b3267de0f417f6c0e05bab87bd51d32839f02644a5357c537` | admin | grant_asset(CBCWU5..) | **new** |
| 268 | 2026-11-17T22:25:02Z | `8919c39b4739c342c36ca7b3cee2ba4b8e132095caecfc3911f82d978ea8a4e4` | admin | grant_asset(CBCWU5..) | **new** |
| 269 | 2026-11-17T22:25:22Z | `0fd2c73c744c117eff3cc47e421eca95e41134c3fdbae6358c72c5b50b2370e2` | admin | grant_asset(CBCWU5..) | **new** |
| 270 | 2026-11-17T22:25:32Z | `53d25c6d0077d80cead5ef5dae5402b57181260638dbad28e8c814ec46d0f3f9` | admin | grant_asset(CBCWU5..) | **new** |
| 271 | 2026-11-17T22:26:27Z | `e3d93e4d98d6f3bd290cdaf20d56b97bf5fe931597b455ea443b053792724fe0` | principal-treasury | change_trust MOCKXAUM | yes |
| 272 | 2026-11-17T22:26:57Z | `fb6d8737a4bb92e529750deff088cbc0223cd0e2082e96ac2e593019964a9314` | xaum-issuer | set_authorized(CBAJVV..) | yes |
| 273 | 2026-11-17T22:27:07Z | `d944bb2f7f0ec051d00755b47b38a4fbdc8fe1ec3eb7ea887b70599de8979f9b` | admin | grant_account(CBCWU5..) | yes |
| 274 | 2026-11-17T22:30:02Z | `1c092628e2d14755ef7466cac09a442d9ca10d69ceb5e7cd62c377234f1d9c8a` | test-a, xaum-issuer | payment 1.0000000 MOCKXAUM GC6VP3..→GCOMBI.. | yes |
| 275 | 2026-11-17T22:31:37Z | `7c0ee5d8700c1be029b33739de3830904206245353f75084d234e96e5b0b12b9` | test-a | deposit(CB7KQ4..) | yes |
| 276 | 2026-11-17T22:31:57Z | `6e65dec17d7c646ad8eb3d649fda66c4fa9ae2cd322ac7bc3c341aca7443c6e8` | test-a | mint(CAR65M..) | yes |
