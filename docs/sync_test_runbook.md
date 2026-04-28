# Sync Subsystem Manual Test Runbook

**Version:** 1.0  
**Last Updated:** 2026-04-28  
**Scope:** Cross-machine sync, JWT refresh, conflict resolution, account deletion

---

## Prerequisites

### Infrastructure

- Two machines (or VMs) referred to as **Machine A** and **Machine B** throughout this runbook. Both must be able to reach the sync server over the network.
- A running Solitaire Quest sync server reachable at a known URL, e.g. `https://solitaire.example.com`. See `README_SERVER.md` for setup.
- Verify the server is live before starting:

  ```bash
  curl -s https://solitaire.example.com/health
  # Expected: {"status":"ok","version":"..."}
  ```

### Accounts

- You will register two separate accounts (`alice` and `bob`) during the tests. You do not need to create them in advance.

### Tooling

- `curl` or a REST client (Insomnia/Postman) for manual API calls.
- `sqlite3` CLI if you need to inspect the server database directly.
- The game binary built in release mode on both machines:

  ```bash
  cargo build -p solitaire_app --release
  ```

### Baseline: Clear local data on both machines

Before starting, delete any existing local save files to ensure a clean state:

```
# Linux
rm -rf ~/.local/share/solitaire_quest/

# macOS
rm -rf ~/Library/Application\ Support/solitaire_quest/

# Windows
rmdir /s %APPDATA%\solitaire_quest\
```

---

## Test 1 — Full Sync Round-Trip (register, play, push, verify on second machine)

**Goal:** Confirm that stats played on Machine A appear on Machine B after sync.

### Step 1 — Register on Machine A

1. Launch the game on Machine A.
2. Open **Settings** (key: `O`) and locate the **Sync** section.
3. Enter the server URL and choose a username: `alice`.
4. Choose a password (at least 12 characters).
5. Tap **Register** (or **Login** if the account already exists).
6. The Settings screen should show **Status: syncing…** briefly, then **Status: last synced at HH:MM**.
7. Close the game.

Verify the registration succeeded directly:

```bash
curl -s -X POST https://solitaire.example.com/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"<your-password>"}' | jq .
# Expected: {"access_token":"...","refresh_token":"..."}
```

### Step 2 — Play games on Machine A

1. Launch the game on Machine A.
2. Win at least **three games** (Draw One or Draw Three — note which mode).
3. Check the Stats overlay (key: `S`) and note:
   - `games_played`
   - `games_won`
   - `win_streak_current`
   - `fastest_win_seconds`
4. Close the game normally (this triggers the push-on-exit path).

### Step 3 — Verify the push reached the server

```bash
# Log in to get a fresh token
TOKEN=$(curl -s -X POST https://solitaire.example.com/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"<your-password>"}' | jq -r .access_token)

# Pull the server's stored state
curl -s -H "Authorization: Bearer $TOKEN" \
  https://solitaire.example.com/api/sync/pull | jq .merged.stats
```

Confirm `games_won` matches what you recorded in Step 2.

### Step 4 — Pull on Machine B

1. Launch the game on **Machine B** (clean local data).
2. Open **Settings**, enter the same server URL, and log in as `alice` with the same password.
3. The plugin will pull on startup. Wait for **Status: last synced at HH:MM**.
4. Open the Stats overlay (key: `S`) and confirm the numbers from Step 2 are present.

**Pass criterion:** `games_won`, `games_played`, and `fastest_win_seconds` on Machine B match Machine A.

---

## Test 2 — JWT Refresh on 401

**Goal:** Confirm that an expired access token is refreshed transparently without user interaction.

### Step 1 — Shorten the access token TTL on the server (test environment only)

Edit the server `.env` and set a short expiry, then restart:

```
JWT_ACCESS_EXPIRY_SECS=5
```

> If you cannot modify the server config, skip to the manual token corruption method in Step 1b.

### Step 1b (alternative) — Corrupt the stored access token directly

On the machine where you want to test (Linux example):

```bash
# List keychain entries (uses secret-tool on GNOME)
secret-tool search service solitaire_quest_server

# Overwrite alice's access token with a deliberately invalid value
secret-tool store --label="alice_access" service solitaire_quest_server account alice_access <<< "invalid.token.value"
```

### Step 2 — Trigger a sync with the expired/invalid token

1. Launch the game.
2. Either wait for the startup pull (for the short-TTL method), or open **Settings** and tap **Sync Now**.
3. Observe the **Status** field.

**Pass criterion (transparent refresh):** Status briefly shows "syncing…" and then shows "last synced at HH:MM" — no auth error is displayed. The access token in the keychain has been silently replaced.

**Verify the new token is valid:**

```bash
# Extract the new token from the keychain
secret-tool lookup service solitaire_quest_server account alice_access | head -c 50
# Should look like a valid JWT (three base64 segments separated by dots)
```

### Step 3 — Test failed refresh (both tokens expired)

1. Corrupt both the access token and the refresh token in the keychain:

   ```bash
   secret-tool store --label="alice_access" service solitaire_quest_server account alice_access <<< "bad"
   secret-tool store --label="alice_refresh" service solitaire_quest_server account alice_refresh <<< "bad"
   ```

2. Launch the game and trigger a sync.

**Pass criterion:** The Settings screen shows an error message matching: "Login expired — tap Sync Now after re-logging in". The game must not crash. No data must be lost (local files are untouched).

3. Restore: log in again via Settings to get fresh tokens.

---

## Test 3 — Conflict Scenario (offline play on both machines, then sync)

**Goal:** Confirm that progress made on both devices offline is merged correctly, with no data silently discarded.

### Step 1 — Take both machines offline

Disable network on both Machine A and Machine B (e.g. airplane mode, or block the server URL in `/etc/hosts`).

### Step 2 — Play on Machine A (offline)

1. Win 5 games. Note the resulting streak and `games_won`.
2. Close the game.

### Step 3 — Play on Machine B (offline)

1. Win 3 different games. Note the resulting streak and `games_won`.
2. Close the game.

At this point Machine A and Machine B have divergent state.

### Step 4 — Re-enable network, sync Machine A first

1. Restore network.
2. Launch the game on Machine A. The push-on-exit from Step 2 did not reach the server, so:
   - Open Settings, tap **Sync Now** to force a pull.
   - Close the game (triggers push-on-exit).
3. Verify the server has Machine A's state:

   ```bash
   curl -s -H "Authorization: Bearer $TOKEN" \
     https://solitaire.example.com/api/sync/pull | jq '.merged.stats.games_won'
   ```

### Step 5 — Sync Machine B

1. Launch the game on Machine B.
2. The startup pull fetches the server's merged state (which now contains Machine A's wins).
3. Open Settings — wait for **Status: last synced at HH:MM**.
4. Open the Stats overlay.

**Pass criteria:**
- `games_won` = max(Machine A wins, Machine B wins) — at minimum the higher of the two counts.
- No games are lost — both machines' win counts contribute.
- If the two machines had different `win_streak_current` values, a conflict should be recorded (visible if you inspect the server response directly):

  ```bash
  curl -s -H "Authorization: Bearer $TOKEN" \
    https://solitaire.example.com/api/sync/pull | jq '.conflicts'
  ```

- The `win_streak_current` conflict entry will show `local_value` and `remote_value`. The higher value is used as the best-effort resolution.

---

## Test 4 — Account Deletion

**Goal:** Confirm that `DELETE /api/account` removes all server-side data and that a subsequent authenticated request is rejected.

### Step 1 — Confirm data exists before deletion

```bash
curl -s -H "Authorization: Bearer $TOKEN" \
  https://solitaire.example.com/api/sync/pull | jq '.merged.stats.games_played'
# Expected: a non-zero number
```

### Step 2 — Delete the account via the API

```bash
curl -s -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  https://solitaire.example.com/api/account | jq .
# Expected: {"ok":true}
```

### Step 3 — Verify all data is gone from the server

```bash
# Try to pull with the (now-invalid) token
curl -s -H "Authorization: Bearer $TOKEN" \
  https://solitaire.example.com/api/sync/pull
# Expected: HTTP 401 Unauthorized

# Try to log in again with the same credentials
curl -s -X POST https://solitaire.example.com/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"<your-password>"}' | jq .
# Expected: HTTP 401 or error body indicating invalid credentials
```

### Step 4 — Verify local data is NOT deleted

1. Open the game. The local files (`stats.json`, `progress.json`, etc.) must still be present and intact — account deletion only affects the server.
2. Check the Stats overlay and confirm local game history is visible.
3. The Settings screen may show an auth error on next sync attempt, which is expected.

### Step 5 — Re-register with the same username (optional)

```bash
curl -s -X POST https://solitaire.example.com/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"<new-password>"}' | jq .
# Expected: {"access_token":"...","refresh_token":"..."} — fresh empty account
```

**Pass criterion:** Re-registration succeeds, and a subsequent pull returns a payload with all-zero stats (completely fresh account, no residual data from the deleted account).

---

## Test 5 — Server Errors Do Not Show "Login Expired"

**Goal:** Verify that a 500 Internal Server Error or 429 Too Many Requests shows a network error, not an auth error, to the user.

### Step 1 — Simulate a 500 with a reverse proxy rule

Add a temporary nginx/Caddy rule to return 500 for `/api/sync/*`:

```nginx
location /api/sync/ {
    return 500;
}
```

Or use a local proxy like `mitmproxy` to intercept and rewrite responses.

### Step 2 — Trigger a sync

Open Settings and tap **Sync Now**.

**Pass criterion:** The Status field shows "Can't reach server — check your connection" (network error message), NOT "Login expired — tap Sync Now after re-logging in" (auth error message).

Remove the nginx rule after this test.

---

## Regression Checklist

After running all tests above, confirm:

- [ ] No crash occurred during any test on either machine.
- [ ] Local save files (`stats.json`, `progress.json`, `achievements.json`) are present and valid JSON after all tests.
- [ ] The game launches and plays normally after all sync operations (sync is additive — never blocks gameplay).
- [ ] The Stats overlay shows correct numbers on both machines after a successful sync round-trip.
- [ ] An expired token is refreshed transparently without the user having to log in again.
- [ ] A doubly-expired token surfaces a clear error message to the user.
- [ ] Account deletion removes all server data; local data is preserved.
- [ ] HTTP 5xx and 429 responses show a network error, not an auth error.
