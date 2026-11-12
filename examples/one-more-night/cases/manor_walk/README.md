# A walk through the manor

An open case over the manor's level manifest, `assets/manor/export/level_manifest.ron`,
that fills the house and solves nothing: ten characters, one of each figurine,
no props, no facts, every feature off. It exists so the house can be walked,
its doors tried and its people met before the real case is written.

| Who | Where | Asked by |
|---|---|---|
| Lady Marchbank | Guest bedroom 1 until seven, then the gallery | Guest |
| Mr Finch and Miss Vale | The drawing room | Guest |
| Colonel Rudge and Dr Amory | The billiard room | Guest |
| Pike, the butler | The dining room sideboard | Staff |
| Mrs Dunn, the cook | The kitchen | Staff |
| Tom, the footman | The servants' hall | Staff |
| Mrs Calder, the housekeeper | Her room | Staff |
| Stokes, the chauffeur | The garage | Staff |

Lady Marchbank shows the rule for private rooms: until seven the Guest may
visit her and the Staff may not walk in; once she has gone out, it is the
other way round.

```sh
cargo run -p one-more-night -- --manor
```
