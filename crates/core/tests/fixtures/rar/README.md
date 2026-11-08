# RAR fixtures

Real archives made by RARLAB `rar` (trial builds). RAR 7 cannot create RAR4, so the RAR4
fixtures come from RAR 6.24 and the RAR5 ones from RAR 7.23. Every `a.txt` holds `hello`.

| File | Tool | Command | Content |
|---|---|---|---|
| `v4.rar` | RAR 6.24 | `rar a -ma4 -ep v4.rar a.txt` | `a.txt` = `hello` |
| `v5.rar` | RAR 7.23 | `rar a -ma5 -ep v5.rar a.txt` | `a.txt` = `hello` |
| `solid.rar` | RAR 7.23 | `rar a -s -ma5 -ep solid.rar a.txt b.txt` | `a.txt` = `hello`, `b.txt` = `goodbye` |
| `v4secret.rar` | RAR 6.24 | `rar a -ma4 -ep -ps3cret-Ünï v4secret.rar a.txt` | `a.txt` = `hello`, data encrypted, password `s3cret-Ünï` |
| `secret.rar` | RAR 7.23 | `rar a -ma5 -ep -ps3cret-Ünï secret.rar note.txt` | `note.txt` = `top secret`, data encrypted, password `s3cret-Ünï` |
| `hidden.rar` | RAR 7.23 | `rar a -ma5 -ep -hps3cret-Ünï hidden.rar note.txt` | `note.txt` = `top secret`, data and file names encrypted, password `s3cret-Ünï` |
