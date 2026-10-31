<?php
/**
 * Ultra-lightweight resilient reverse proxy gateway with on-demand auto-spawn
 * and multi-channel authentication (Header, Bearer, Cookie, URL Token, Basic Auth)
 * for dbsrt-server on o2switch cPanel Cloud.
 *
 * Relays public HTTPS requests from Apache/LiteSpeed to the local Rust dbsrt-server daemon
 * listening on the Unix socket ~/dbsrt/run/dbsrt-server.sock without requiring Apache
 * mod_proxy privileges (o2switch kills TCP listeners within seconds, not Unix sockets).
 * Automatically spawns the daemon if inactive or harvested by CloudLinux.
 * Provides mobile-friendly login interface, rate-limiting brute-force defense,
 * and timing-safe authentication for remote monitoring over 4G/5G mobile.
 */

declare(strict_types=1);

// Operational configuration constants
const UPSTREAM_BASE_URL = 'http://localhost';
const UPSTREAM_SOCKET_REL_PATH = '/dbsrt/run/dbsrt-server.sock';
const CONNECT_TIMEOUT_SECONDS = 2;
const EXEC_TIMEOUT_SECONDS = 30;
const SPAWN_MAX_RETRIES = 6;
const SPAWN_RETRY_INTERVAL_MICROSECONDS = 250000; // 250 ms
const SECRET_KEY_REL_PATH = '/dbsrt/etc/api_key.secret';
const READONLY_KEY_REL_PATH = '/dbsrt/etc/api_key_readonly.secret';
const SPAWN_LOCK_REL_PATH = '/dbsrt/run/gateway_spawn.lock';
// Set by `o2switch_ctl.sh stop` (dbsrt_server_ctl.sh), under the spawn lock: no auto-spawn
const STOP_MARKER_REL_PATH = '/dbsrt/run/dbsrt-server.stopped';
const RATE_LIMIT_REL_PATH = '/dbsrt/run/auth_rate_limit.json';
const AUTH_COOKIE_NAME = 'dbsrt_session';
const AUTH_COOKIE_MAX_AGE = 2592000; // 30 days
const SESSION_SALT = 'dbsrt_session_v1_';
const MAX_FAILED_ATTEMPTS = 5;
const RATE_LIMIT_WINDOW_SECONDS = 900; // 15 min lockout

// Derive user home directory relative to public_html/dbsrt
$homeDir = dirname(__DIR__, 2);
$secretKeyFile = $homeDir . SECRET_KEY_REL_PATH;
$readOnlyKeyFile = $homeDir . READONLY_KEY_REL_PATH;
$rateLimitFile = $homeDir . RATE_LIMIT_REL_PATH;

/**
 * Validates request authentication against configured API secret keys.
 * Supports multi-key pools (one key per line) for zero-downtime key rotation.
 */
function load_valid_keys(string $secretKeyFile): array
{
    if (!is_file($secretKeyFile) || !is_readable($secretKeyFile)) {
        return [];
    }

    $validKeys = [];
    foreach (explode("\n", (string) file_get_contents($secretKeyFile)) as $line) {
        $key = trim($line);
        if ($key !== '' && !str_starts_with($key, '#')) {
            $validKeys[] = $key;
        }
    }
    return $validKeys;
}

function compute_session_hash(string $secretKey): string
{
    return hash('sha256', SESSION_SALT . $secretKey);
}

function is_ip_rate_limited(string $rateLimitFile, string $clientIp): bool
{
    if (!is_file($rateLimitFile)) {
        return false;
    }
    $data = json_decode((string) file_get_contents($rateLimitFile), true);
    if (!is_array($data) || !isset($data[$clientIp])) {
        return false;
    }
    $entry = $data[$clientIp];
    $attempts = (int) ($entry['attempts'] ?? 0);
    $lastAttempt = (int) ($entry['time'] ?? 0);

    if (time() - $lastAttempt > RATE_LIMIT_WINDOW_SECONDS) {
        return false;
    }
    return $attempts >= MAX_FAILED_ATTEMPTS;
}

function record_auth_failure(string $rateLimitFile, string $clientIp): void
{
    $data = is_file($rateLimitFile) ? (json_decode((string) file_get_contents($rateLimitFile), true) ?: []) : [];
    $entry = $data[$clientIp] ?? ['attempts' => 0, 'time' => time()];
    if (time() - (int)$entry['time'] > RATE_LIMIT_WINDOW_SECONDS) {
        $entry['attempts'] = 0;
    }
    $entry['attempts'] = ((int)$entry['attempts']) + 1;
    $entry['time'] = time();
    $data[$clientIp] = $entry;

    $tmp = $rateLimitFile . '.tmp';
    @file_put_contents($tmp, json_encode($data));
    @rename($tmp, $rateLimitFile);
}

function clear_auth_failures(string $rateLimitFile, string $clientIp): void
{
    if (!is_file($rateLimitFile)) {
        return;
    }
    $data = json_decode((string) file_get_contents($rateLimitFile), true);
    if (is_array($data) && isset($data[$clientIp])) {
        unset($data[$clientIp]);
        $tmp = $rateLimitFile . '.tmp';
        @file_put_contents($tmp, json_encode($data));
        @rename($tmp, $rateLimitFile);
    }
}

function check_key_match(string $candidate, array $validKeys): bool
{
    if ($candidate === '') {
        return false;
    }
    foreach ($validKeys as $expectedKey) {
        if (hash_equals($expectedKey, $candidate)) {
            return true;
        }
    }
    return false;
}

function set_auth_session_cookie(string $key): void
{
    $hash = compute_session_hash($key);
    $isHttps = (!empty($_SERVER['HTTPS']) && $_SERVER['HTTPS'] !== 'off') ||
               ($_SERVER['SERVER_PORT'] ?? 80) == 443 ||
               (isset($_SERVER['HTTP_X_FORWARDED_PROTO']) && $_SERVER['HTTP_X_FORWARDED_PROTO'] === 'https');

    setcookie(AUTH_COOKIE_NAME, $hash, [
        'expires' => time() + AUTH_COOKIE_MAX_AGE,
        'path' => '/',
        'domain' => '',
        'secure' => $isHttps,
        'httponly' => true,
        'samesite' => 'Lax',
    ]);
}

function is_authenticated(array $validKeys, string &$matchedKey = ''): bool
{
    $headers = function_exists('getallheaders') ? (getallheaders() ?: []) : [];

    // 1. Check X-API-Key header (via $_SERVER or getallheaders)
    $candidate = $_SERVER['HTTP_X_API_KEY'] ?? '';
    if ($candidate === '') {
        foreach ($headers as $name => $value) {
            if (strcasecmp($name, 'X-API-Key') === 0) {
                $candidate = trim((string) $value);
                break;
            }
        }
    }
    if ($candidate !== '' && check_key_match($candidate, $validKeys)) {
        $matchedKey = $candidate;
        return true;
    }

    // 2. Check Authorization: Bearer <token>
    $authHeader = $headers['Authorization'] ?? $headers['authorization'] ?? $_SERVER['HTTP_AUTHORIZATION'] ?? '';
    if (preg_match('/^Bearer\s+(\S+)$/i', trim((string) $authHeader), $matches)) {
        $candidate = $matches[1];
        if (check_key_match($candidate, $validKeys)) {
            $matchedKey = $candidate;
            return true;
        }
    }

    // 3. Check HTTP Basic Auth (user or password matching the key)
    $basicUser = $_SERVER['PHP_AUTH_USER'] ?? '';
    $basicPw = $_SERVER['PHP_AUTH_PW'] ?? '';
    if (check_key_match($basicPw, $validKeys) || check_key_match($basicUser, $validKeys)) {
        $matchedKey = check_key_match($basicPw, $validKeys) ? $basicPw : $basicUser;
        return true;
    }

    // 4. Check session cookie
    $cookieHash = $_COOKIE[AUTH_COOKIE_NAME] ?? '';
    if ($cookieHash !== '') {
        foreach ($validKeys as $expectedKey) {
            if (hash_equals(compute_session_hash($expectedKey), $cookieHash)) {
                $matchedKey = $expectedKey;
                return true;
            }
        }
    }

    // 5. Check URL query token (?token=... or ?key=...)
    $queryToken = $_GET['token'] ?? $_GET['key'] ?? '';
    if ($queryToken !== '') {
        if (check_key_match($queryToken, $validKeys)) {
            $matchedKey = $queryToken;
            set_auth_session_cookie($queryToken);
            return true;
        }
    }

    return false;
}

function is_daemon_healthy(string $homeDir, int $connectTimeoutMs, int $timeoutMs): bool
{
    $ch = curl_init(UPSTREAM_BASE_URL . '/v1/health');
    curl_setopt($ch, CURLOPT_UNIX_SOCKET_PATH, $homeDir . UPSTREAM_SOCKET_REL_PATH);
    curl_setopt($ch, CURLOPT_CONNECTTIMEOUT_MS, $connectTimeoutMs);
    curl_setopt($ch, CURLOPT_TIMEOUT_MS, $timeoutMs);
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    $res = curl_exec($ch);
    $code = curl_getinfo($ch, CURLINFO_HTTP_CODE);
    curl_close($ch);
    return $res !== false && $code === 200;
}

/**
 * Spawns the dbsrt-server daemon (called only when the upstream refused the connection).
 * An exclusive lock serializes concurrent requests so only one of them spawns the daemon.
 */
function spawn_daemon(string $homeDir): bool
{
    // 'e' (close-on-exec): the spawned daemon must not inherit the lock, or it would hold it for
    // its whole life if this request dies before the unlock below
    $lock = fopen($homeDir . SPAWN_LOCK_REL_PATH, 'ce');
    if ($lock === false || !flock($lock, LOCK_EX)) {
        return false;
    }

    try {
        if (is_daemon_healthy($homeDir, 1500, 2000)) {
            return true;
        }
        if (file_exists($homeDir . STOP_MARKER_REL_PATH)) {
            return false;
        }

        $baseDir = escapeshellarg($homeDir . '/dbsrt');
        $bin = escapeshellarg($homeDir . '/dbsrt/bin/dbsrt-server');
        $catalog = escapeshellarg($homeDir . '/dbsrt/data/catalog.dbphash');
        $srtDir = escapeshellarg($homeDir . '/dbsrt/data/SRT');
        $phashDir = escapeshellarg($homeDir . '/dbsrt/run/empty_phash');
        $log = escapeshellarg($homeDir . '/dbsrt/logs/dbsrt-server.log');

        $keyFile = escapeshellarg($homeDir . SECRET_KEY_REL_PATH);
        $socket = escapeshellarg($homeDir . UPSTREAM_SOCKET_REL_PATH);

        // Redirect the whole backgrounded subshell, then exec: no shell is left holding exec()'s
        // stdout pipe, which would block this request until PHP kills it
        exec(sprintf(
            '(cd %s && exec setsid %s --unix-socket %s --dbphash %s --subtitles-dir %s --phash-dir %s --api-key-file %s) </dev/null >> %s 2>&1 &',
            $baseDir,
            $bin,
            $socket,
            $catalog,
            $srtDir,
            $phashDir,
            $keyFile,
            $log
        ));

        for ($i = 0; $i < SPAWN_MAX_RETRIES; $i++) {
            usleep(SPAWN_RETRY_INTERVAL_MICROSECONDS);
            if (is_daemon_healthy($homeDir, 1000, 1500)) {
                return true;
            }
        }
        return false;
    } finally {
        flock($lock, LOCK_UN);
        fclose($lock);
    }
}

function send_json_error(int $code, array $payload): void
{
    http_response_code($code);
    header('Content-Type: application/json; charset=utf-8');
    echo json_encode($payload, JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT);
    exit;
}

function render_login_page(?string $errorMessage = null): void
{
    http_response_code(401);
    header('Content-Type: text/html; charset=utf-8');
?>
<!DOCTYPE html>
<html lang="fr">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
  <meta name="theme-color" content="#0d1117">
  <title>Connexion ── dbsrt Monitor Cloud</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: #0d1117;
      color: #c9d1d9;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      padding: 16px;
    }
    .card {
      background: #161b22;
      border: 1px solid #30363d;
      border-radius: 8px;
      padding: 28px 24px;
      width: 100%;
      max-width: 380px;
      box-shadow: 0 8px 24px rgba(0,0,0,0.5);
    }
    h1 {
      font-size: 1.25rem;
      color: #fff;
      display: flex;
      align-items: center;
      gap: 8px;
      margin-bottom: 8px;
    }
    p {
      font-size: 0.85rem;
      color: #8b949e;
      margin-bottom: 20px;
      line-height: 1.4;
    }
    .err-box {
      background: rgba(248, 81, 73, 0.15);
      border: 1px solid #f85149;
      color: #f85149;
      padding: 10px 12px;
      border-radius: 6px;
      font-size: 0.82rem;
      margin-bottom: 16px;
    }
    .input-field {
      width: 100%;
      background: #090d13;
      border: 1px solid #30363d;
      color: #fff;
      padding: 12px;
      border-radius: 6px;
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
      font-size: 0.95rem;
      margin-bottom: 16px;
    }
    .input-field:focus {
      outline: none;
      border-color: #58a6ff;
    }
    .btn-submit {
      width: 100%;
      background: #1f6feb;
      border: 1px solid #388bfd;
      color: #fff;
      padding: 12px;
      border-radius: 6px;
      font-size: 0.95rem;
      font-weight: 600;
      cursor: pointer;
    }
    .btn-submit:hover { background: #388bfd; }
  </style>
</head>
<body>
  <div class="card">
    <h1>📡 dbsrt Monitor</h1>
    <p>Accès public sécurisé 4G/5G. Veuillez saisir la clé secrète API pour ouvrir la session de télémétrie.</p>
    <?php if ($errorMessage): ?>
      <div class="err-box"><?= htmlspecialchars($errorMessage, ENT_QUOTES, 'UTF-8') ?></div>
    <?php endif; ?>
    <form method="POST" action="">
      <input type="password" name="api_key" class="input-field" placeholder="Clé Secrète API" required autofocus>
      <button type="submit" class="btn-submit">Déverrouiller l'accès</button>
    </form>
  </div>
</body>
</html>
<?php
    exit;
}

// Handle CORS preflight for browser WebAssembly clients
if (($_SERVER['REQUEST_METHOD'] ?? 'GET') === 'OPTIONS') {
    header('Access-Control-Allow-Origin: *');
    header('Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS');
    header('Access-Control-Allow-Headers: Content-Type, Authorization, X-API-Key');
    http_response_code(204);
    exit;
}

// 1. Load Secret Keys: master pool (full access) + optional read-only pool (monitoring GETs)
$masterKeys = load_valid_keys($secretKeyFile);
if ($masterKeys === []) {
    send_json_error(503, ['error' => 'Service Unavailable: gateway API key not provisioned']);
}
$validKeys = array_merge($masterKeys, load_valid_keys($readOnlyKeyFile));

$clientIp = $_SERVER['HTTP_X_FORWARDED_FOR'] ?? $_SERVER['REMOTE_ADDR'] ?? '127.0.0.1';
if (str_contains($clientIp, ',')) {
    $clientIp = trim(explode(',', $clientIp)[0]);
}

// Check Rate Limiting
if (is_ip_rate_limited($rateLimitFile, $clientIp)) {
    send_json_error(429, [
        'error' => 'Too Many Requests',
        'message' => 'Rate limit exceeded: 5 failed attempts. Locked for 15 minutes.',
    ]);
}

// Handle Logout
$requestUri = $_SERVER['REQUEST_URI'] ?? '/';
if (str_contains($requestUri, '/logout')) {
    setcookie(AUTH_COOKIE_NAME, '', ['expires' => time() - 3600, 'path' => '/']);
    header('Location: monitoring');
    exit;
}

// Handle Form POST Login
if (($_SERVER['REQUEST_METHOD'] ?? 'GET') === 'POST' && isset($_POST['api_key'])) {
    $submittedKey = trim((string) $_POST['api_key']);
    if (check_key_match($submittedKey, $validKeys)) {
        clear_auth_failures($rateLimitFile, $clientIp);
        set_auth_session_cookie($submittedKey);
        // Redirect to monitoring page
        header('Location: ' . strtok($requestUri, '?'));
        exit;
    } else {
        record_auth_failure($rateLimitFile, $clientIp);
        render_login_page('Clé secrète invalide. Tentative enregistrée.');
    }
}

// 2. Strict Authentication Check
$matchedKey = '';
if (!is_authenticated($validKeys, $matchedKey)) {
    record_auth_failure($rateLimitFile, $clientIp);
    $acceptHeader = $_SERVER['HTTP_ACCEPT'] ?? '';
    if (str_contains($acceptHeader, 'text/html')) {
        render_login_page();
    } else {
        send_json_error(401, [
            'error' => 'Unauthorized',
            'message' => 'Valid API key required via X-API-Key header, Bearer token, session cookie, or query token.',
        ]);
    }
}

// On successful authentication, clear any prior rate limit failures
clear_auth_failures($rateLimitFile, $clientIp);

// 3. Determine requested sub-path
$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$scriptName = $_SERVER['SCRIPT_NAME'] ?? '';
$scriptDir = dirname($scriptName);

$path = parse_url($requestUri, PHP_URL_PATH) ?? '/';
if ($scriptDir !== '/' && str_starts_with($path, $scriptDir)) {
    $path = substr($path, strlen($scriptDir));
}
if (!str_starts_with($path, '/')) {
    $path = '/' . $path;
}

// Default root or empty path to /monitoring
if ($path === '/' || $path === '') {
    $path = '/monitoring';
}

// Read-only keys (web-top QR, shared dashboards) only reach monitoring GETs
$isReadOnly = !check_key_match($matchedKey, $masterKeys);
$readOnlyPathAllowed = in_array($path, ['/monitoring', '/top', '/health', '/v1/health'], true)
    || str_starts_with($path, '/v1/monitoring/');
if ($isReadOnly && (!in_array($method, ['GET', 'HEAD'], true) || !$readOnlyPathAllowed)) {
    send_json_error(403, ['error' => 'Forbidden: read-only key']);
}

$upstreamQuery = $_SERVER['QUERY_STRING'] ?? '';
$upstreamUrl = sprintf('%s%s%s', UPSTREAM_BASE_URL, $path, $upstreamQuery !== '' ? '?' . $upstreamQuery : '');

// Forward request headers, minus hop-by-hop, client forwarding and client credential headers:
// the upstream only ever sees the master key, so a read-only key cannot reach it directly
$droppedHeaders = [
    'host', 'content-length', 'connection', 'x-forwarded-for', 'x-forwarded-proto',
    'authorization', 'x-api-key', 'cookie',
];
$forwardHeaders = [];
foreach (getallheaders() as $name => $value) {
    if (!in_array(strtolower($name), $droppedHeaders, true)) {
        $forwardHeaders[] = sprintf('%s: %s', $name, $value);
    }
}
$forwardHeaders[] = sprintf('X-Forwarded-For: %s', $clientIp);
$forwardHeaders[] = 'X-Forwarded-Proto: https';
$forwardHeaders[] = sprintf('Authorization: Bearer %s', $masterKeys[0]);

$requestBody = in_array($method, ['POST', 'PUT', 'PATCH'], true) ? file_get_contents('php://input') : false;

$ch = curl_init($upstreamUrl);
curl_setopt($ch, CURLOPT_UNIX_SOCKET_PATH, $homeDir . UPSTREAM_SOCKET_REL_PATH);
curl_setopt($ch, CURLOPT_CUSTOMREQUEST, $method);
curl_setopt($ch, CURLOPT_NOBODY, $method === 'HEAD');
curl_setopt($ch, CURLOPT_HTTPHEADER, $forwardHeaders);
curl_setopt($ch, CURLOPT_CONNECTTIMEOUT, CONNECT_TIMEOUT_SECONDS);
curl_setopt($ch, CURLOPT_TIMEOUT, EXEC_TIMEOUT_SECONDS);
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
curl_setopt($ch, CURLOPT_HEADER, true);
if ($requestBody !== false && $requestBody !== '') {
    curl_setopt($ch, CURLOPT_POSTFIELDS, $requestBody);
}

$rawResponse = curl_exec($ch);

// Lazy on-demand activation: spawn only when the daemon refused the connection
if ($rawResponse === false && curl_errno($ch) === CURLE_COULDNT_CONNECT) {
    if (!spawn_daemon($homeDir)) {
        curl_close($ch);
        if (file_exists($homeDir . STOP_MARKER_REL_PATH)) {
            send_json_error(503, ['error' => 'Service Unavailable: dbsrt-server stopped by operator']);
        }
        send_json_error(502, ['error' => 'Bad Gateway: Unable to reach or spawn dbsrt-server']);
    }
    $rawResponse = curl_exec($ch);
}

if ($rawResponse === false) {
    error_log('dbsrt gateway upstream error: ' . curl_error($ch));
    curl_close($ch);
    send_json_error(502, ['error' => 'Bad Gateway: Upstream request failed']);
}

$headerSize = curl_getinfo($ch, CURLINFO_HEADER_SIZE);
$httpCode = curl_getinfo($ch, CURLINFO_HTTP_CODE);
curl_close($ch);

$rawHeaders = substr($rawResponse, 0, $headerSize);
$body = substr($rawResponse, $headerSize);

http_response_code($httpCode);

// Sanitize internal paths in /v1/status to avoid information disclosure
if (str_ends_with($path, '/status')) {
    $statusData = json_decode($body, true);
    if (is_array($statusData)) {
        unset($statusData['phash_dir'], $statusData['subtitles_dir']);
        $body = (string) json_encode($statusData, JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT);
    }
}

// Forward response headers
foreach (explode("\r\n", $rawHeaders) as $headerLine) {
    if (empty($headerLine) || str_starts_with(strtoupper($headerLine), 'HTTP/')) {
        continue;
    }
    $parts = explode(':', $headerLine, 2);
    if (count($parts) === 2) {
        $headerName = strtolower(trim($parts[0]));
        if ($headerName === 'transfer-encoding' || $headerName === 'connection' || $headerName === 'content-length') {
            continue;
        }
        header($headerLine, true);
    }
}

header('Content-Length: ' . strlen($body));
echo $body;
