# dab.rs
A Unix-style command-line music player built with Rust, featuring streaming playback, local caching, and a terminal user interface inspired by cmus and vim.

```
┌─ DAB Music Player ─────────────────────────────────────────────────────────────────────┐
│ ▶ Anthrax - Madhouse: The Very Best Of Anthrax - Madhouse [02:25/04:17]                │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ Favorite Albums (16) │ Enter: Play │ l: Album detail │ h: Artist discography │ a: Add  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ Saxon - Strong Arm Of The Law (Edition spéciale) (Unknown year)                        │
│ Anthrax - Madhouse: The Very Best Of Anthrax                                           │
│ Megadeth - Rust In Peace (Unknown year)                                                │
│ Jacqueline du Pré - The Heart of the Cello (Unknown year)                              │
│ Antonio Vivaldi - Great Composers - Vivaldi (Unknown year)                             │
│ Billie Eilish - HIT ME HARD AND SOFT (Unknown year)                                    │
│ Sia - 1000 Forms Of Fear (Deluxe Version) (2014-07-04)                                 │
│ Amorphis - Tuonela (Unknown year)                                                      │
│ Pantera - Cowboys From Hell (2010-04-17)                                               │
│ Judas Priest - Painkiller (1990-08-01)                                                 │
│ Arch Enemy - Deceivers (2022-08-12)                                                    │
│ Opeth - The Last Will And Testament (2024-10-11)                                       │
│ My Dying Bride - 34.788%... Complete (1998-01-01)                                      │
│ Amon Amarth - The Great Heathen Army (2022-08-05)                                      │
│ Coldplay - X&Y (2005-06-06)                                                            │
│ Lana Del Rey - Born To Die (2012-01-30)                                                │
└────────────────────────────────────────────────────────────────────────────────────────┘
Player state: Playing
```

## TUI Controls

| Key | Action |
|-----|--------|
| `1` | goto favorite view |
| `2` | goto queue view |
| `3` | goto search view |
| `Enter` | Play selected track/album |
| `/` | Search for music |
| `j`/`k` | Navigate up/down |
| `l` | Show album details |
| `h` | Show artist discography |
| `a` | Add track to queue (next) |
| `A` | Replace queue with current view's tracks |
| `m` | Add album to favorites |
| `Esc` | Go back to previous view |
| `Space` | Play/pause |
| `n` | Next track |
| `p` | Previous track |
