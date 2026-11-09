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
| `two4.rar` | RAR 6.24 | `rar a -ma4 -ep two4.rar a.txt b.txt` | `a.txt` = `hello`, `b.txt` = `goodbye`, 123 bytes; `a.txt` ends at 67, `b.txt`'s header spans 67 to 109 |
| `dir4.rar` | RAR 6.24 | `rar a -ma4 dir4.rar d a.txt`, `d` holding the empty folder `d/e` | `a.txt` = `hello`, folders `d/e` and `d`, 152 bytes; folder `d/e` ends at 107 |
| `solid4.rar` | RAR 6.24 | `rar a -ma4 -ep -s solid4.rar a.txt b.txt` | solid, `a.txt` = `hello`, `b.txt` = `goodbye`, 133 bytes; `a.txt`'s block ends at 80, `b.txt`'s at 126 |
| `big4.rar` | RAR 6.24 | `rar a -ma4 -ep -m0 big4.rar r.bin a.txt`, `r.bin` = 3 000 bytes of `x` | stored `r.bin` then `a.txt` = `hello`, 3 116 bytes; `r.bin`'s data spans 62 to 3 062 |
| `extras4.rar` | RAR 6.24 | `rar a -ma4 -ep -rr5% -zc.txt -k extras4.rar a.txt b.txt`, `c.txt` = `a comment` | `a.txt` = `hello`, `b.txt` = `goodbye`, locked, with an archive comment (block 20 to 74) and a recovery record (block 170 to 738), 745 bytes |
| `hp5two.rar` | RAR 7.23 | `rar a -ma5 -ep -hps3cret-Ünï hp5two.rar a.txt b.txt` | `a.txt` = `hello`, `b.txt` = `goodbye`, data and file names encrypted, password `s3cret-Ünï`, 382 bytes; encryption header at 8 (lg2_count 15), `a.txt`'s block ends at 206 |
| `hp4.rar` | RAR 6.24 | `rar a -ma4 -ep -hps3cret-Ünï hp4.rar a.txt b.txt` | `a.txt` = `hello`, `b.txt` = `goodbye`, data and file names encrypted, password `s3cret-Ünï`, 220 bytes; one salt for every header; `a.txt`'s header (8 byte salt then 64 bytes) spans 20 to 92 and its data ends at 108, `b.txt`'s data spans 180 to 196, the end block 196 to 220 |
| `hp4long.rar` | RAR 6.24 | `rar a -ma4 -ep -hpCorrectHorseBatteryStapleÜnï0123456789ab hp4long.rar a.txt` | `a.txt` = `hello`, data and file names encrypted, 40 character password `CorrectHorseBatteryStapleÜnï0123456789ab` (long enough for UnRAR's SHA-1 schedule write back), 132 bytes; `a.txt`'s header spans 20 to 92 |
