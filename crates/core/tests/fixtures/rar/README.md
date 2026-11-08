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
| `part.rar` | RAR 7.23 | `rar a -ma5 -ep part.rar a.txt` then `rar a -ma5 -ep -ps3cret-Ünï part.rar b.txt` (the password is given only for the second add) | `a.txt` = `hello` plain, `b.txt` = `bee` data encrypted, password `s3cret-Ünï` |
| `both.rar` | RAR 7.23 | `rar a -ma5 -ep -ol -ps3cret-Ünï both.rar a.txt l`, where `l` is a symbolic link | `a.txt` = `hello`, symbolic link `l`, data encrypted, readable file names, password `s3cret-Ünï` |
| `hl2.rar` | RAR 7.23 | `rar a -ma5 -ep -ol -hps3cret-Ünï hl2.rar a.txt l` (same files as `both.rar`) | data and file names encrypted, symbolic link `l`, password `s3cret-Ünï` |
| `cut.rar` | `head` on `secret.rar` | `head -c 103 secret.rar > cut.rar` (2/3 of the 155 bytes, rounded down) | truncated RAR 5, data encrypted, readable file names |
| `cuth.rar` | `head` on `hidden.rar` | `head -c 169 hidden.rar > cuth.rar` (2/3 of the 254 bytes, rounded down) | truncated RAR 5, data and file names encrypted, password `s3cret-Ünï` |
