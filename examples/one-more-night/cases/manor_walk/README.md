# A walk through the manor

An open case over the manor's level manifest, `assets/manor/export/level_manifest.ron`,
that fills the house and solves nothing: ten characters, one of each figurine,
no props, no facts, every feature off. It exists so the house can be walked,
its doors tried and its people met before the real case is written.

| Who | Their evening | Asked by |
|---|---|---|
| Lady Marchbank | Guest bedroom 1, the gallery at seven, dinner at eight, the drawing room, and up to bed at eleven | Guest |
| Mr Finch | The drawing room, dinner, then the library | Guest |
| Miss Vale | The drawing room, dinner, the drawing room, and up to guest bedroom 3 | Guest |
| Colonel Rudge and Dr Amory | The billiard room, dinner, the billiard room | Guest |
| Pike, the butler | The dining room sideboard, then the pantry | Staff |
| Mrs Dunn, the cook | The kitchen | Staff |
| Tom, the footman | The servants' hall, waiting at dinner, and up the back stairs to the attic at eleven | Staff |
| Mrs Calder, the housekeeper | Her room, with a round of the gallery at ten | Staff |
| Stokes, the chauffeur | The garage, then the servants' hall at nine | Staff |

They walk by the stairs and round the furniture, on the walk graph in the
level manifest.

Lady Marchbank shows the rule for private rooms: until seven the Guest may
visit her and the Staff may not walk in; once she has gone out, it is the
other way round, until she comes back up at eleven.

```sh
cargo run -p one-more-night -- --manor
```
