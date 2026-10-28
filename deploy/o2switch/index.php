<?php
/**
 * Ultra-lightweight resilient reverse proxy gateway with on-demand auto-spawn
 * and API Key authentication for dbsrt-server on o2switch cPanel Cloud.
 *
 * Relays public HTTPS requests from Apache/LiteSpeed to the local Rust dbsrt-server daemon
 * listening on 127.0.0.1:8765 without requiring Apache mod_proxy privileges.
 * Automatically spawns the daemon if inactive or harvested by CloudLinux.
 * Enforces strict API Key authentication (X-API-Key, Bearer token, or api_key query param).
 */

declare(strict_types=1);

// Operational configuration constants
const UPSTREAM_HOST = 'http://127.0.0.1';
const UPSTREAM_PORT = 8765;
const CONNECT_TIMEOUT_SECONDS = 2;
const EXEC_TIMEOUT_SECONDS = 30;
const SPAWN_MAX_RETRIES = 6;
const SPAWN_RETRY_INTERVAL_MICROSECONDS = 250000; // 250 ms
const SECRET_KEY_REL_PATH = '/dbsrt/etc/api_key.secret';

// Derive user home directory relative to public_html/dbsrt
$homeDir = dirname(__DIR__, 2);
$secretKeyFile = $homeDir . SECRET_KEY_REL_PATH;

/**
 * Validates request authentication against configured API secret keys.
 * Supports multi-key pools (one key per line) for zero-downtime key rotation.
 */
function is_authenticated(string $secretKeyFile): bool
{
    if (!is_file($secretKeyFile) || !is_readable($secretKeyFile)) {
        // If no secret key is provisioned, allow open access (fallback)
        return true;
    }

    $rawContent = (string) file_get_contents($secretKeyFile);
    $validKeys = [];
    foreach (explode("\n", $rawContent) as $line) {
        $key = trim($line);
        if ($key !== '' && !str_starts_with($key, '#')) {
            $validKeys[] = $key;
        }
    }

    if (empty($validKeys)) {
        return true;
    }

    // Extract provided client token from headers or query params
    $clientToken = null;
    $headers = getallheaders();

    // 1. Check X-API-Key header
    foreach ($headers as $name => $value) {
        if (strcasecmp($name, 'X-API-Key') === 0) {
            $clientToken = trim((string) $value);
            break;
        }
    }

    // 2. Check Authorization: Bearer <token>
    if ($clientToken === null) {
        $authHeader = $headers['Authorization'] ?? $headers['authorization'] ?? $_SERVER['HTTP_AUTHORIZATION'] ?? '';
        if (preg_match('/^Bearer\s+(\S+)$/i', trim((string) $authHeader), $matches)) {
            $clientToken = $matches[1];
        }
    }

    // 3. Check query parameter ?api_key=<token> (convenient for browser verification)
    if ($clientToken === null && isset($_GET['api_key'])) {
        $clientToken = (string) $_GET['api_key'];
    }

    if ($clientToken === null || $clientToken === '') {
        return false;
    }

    // Timing-safe comparison against all valid keys
    foreach ($validKeys as $expectedKey) {
        if (hash_equals($expectedKey, $clientToken)) {
            return true;
        }
    }

    return false;
}

/**
 * Ensures the dbsrt-server daemon is alive, auto-spawning it if offline.
 */
function ensure_daemon_alive(string $homeDir): bool
{
    $healthUrl = sprintf('%s:%d/v1/health', UPSTREAM_HOST, UPSTREAM_PORT);

    // Quick test if already alive
    $ch = curl_init($healthUrl);
    curl_setopt($ch, CURLOPT_CONNECTTIMEOUT_MS, 1500);
    curl_setopt($ch, CURLOPT_TIMEOUT_MS, 2000);
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    $res = curl_exec($ch);
    $code = curl_getinfo($ch, CURLINFO_HTTP_CODE);
    curl_close($ch);

    if ($res !== false && $code === 200) {
        return true;
    }

    // Spawn server in background detached session
    $bin = escapeshellarg($homeDir . '/dbsrt/bin/dbsrt-server');
    $catalog = escapeshellarg($homeDir . '/dbsrt/data/catalog.dbphash');
    $srtDir = escapeshellarg($homeDir . '/dbsrt/data/SRT');
    $phashDir = escapeshellarg($homeDir . '/dbsrt/data/__PHASH__');
    $log = escapeshellarg($homeDir . '/dbsrt/logs/dbsrt-server.log');

    $cmd = sprintf(
        'setsid %s --host 127.0.0.1 --port %d --dbphash %s --subtitles-dir %s --phash-dir %s </dev/null >> %s 2>&1 &',
        $bin,
        UPSTREAM_PORT,
        $catalog,
        $srtDir,
        $phashDir,
        $log
    );

    exec($cmd);

    // Poll until healthy or retries exhausted
    for ($i = 0; $i < SPAWN_MAX_RETRIES; $i++) {
        usleep(SPAWN_RETRY_INTERVAL_MICROSECONDS);
        $ch = curl_init($healthUrl);
        curl_setopt($ch, CURLOPT_CONNECTTIMEOUT_MS, 1000);
        curl_setopt($ch, CURLOPT_TIMEOUT_MS, 1500);
        curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
        $res = curl_exec($ch);
        $code = curl_getinfo($ch, CURLINFO_HTTP_CODE);
        curl_close($ch);

        if ($res !== false && $code === 200) {
            return true;
        }
    }

    return false;
}

// 1. Strict Authentication Check
if (!is_authenticated($secretKeyFile)) {
    http_response_code(401);
    header('Content-Type: application/json; charset=utf-8');
    echo json_encode([
        'error' => 'Unauthorized',
        'message' => 'Valid API key required via X-API-Key header, Bearer token, or ?api_key= query parameter.',
    ], JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT);
    exit;
}

// 2. Ensure backend is ready (lazy on-demand activation)
if (!ensure_daemon_alive($homeDir)) {
    http_response_code(502);
    header('Content-Type: application/json; charset=utf-8');
    echo json_encode([
        'error' => 'Bad Gateway: Unable to reach or spawn dbsrt-server on 127.0.0.1:' . UPSTREAM_PORT,
    ], JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT);
    exit;
}

// 3. Determine requested sub-path
$requestUri = $_SERVER['REQUEST_URI'] ?? '/';
$scriptName = $_SERVER['SCRIPT_NAME'] ?? '';
$scriptDir = dirname($scriptName);

$path = parse_url($requestUri, PHP_URL_PATH) ?? '/';
if ($scriptDir !== '/' && str_starts_with($path, $scriptDir)) {
    $path = substr($path, strlen($scriptDir));
}
if (!str_starts_with($path, '/')) {
    $path = '/' . $path;
}

// Strip internal ?api_key parameter before forwarding upstream
$queryParams = $_GET;
unset($queryParams['api_key']);
$upstreamQuery = http_build_query($queryParams);
$upstreamUrl = sprintf('%s:%d%s%s', UPSTREAM_HOST, UPSTREAM_PORT, $path, $upstreamQuery !== '' ? '?' . $upstreamQuery : '');

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$ch = curl_init($upstreamUrl);

// Forward request headers
$forwardHeaders = [];
foreach (getallheaders() as $name => $value) {
    $lower = strtolower($name);
    if ($lower === 'host' || $lower === 'content-length' || $lower === 'connection' || $lower === 'x-api-key') {
        continue;
    }
    $forwardHeaders[] = sprintf('%s: %s', $name, $value);
}

$remoteAddr = $_SERVER['REMOTE_ADDR'] ?? '127.0.0.1';
$forwardHeaders[] = sprintf('X-Forwarded-For: %s', $remoteAddr);
$forwardHeaders[] = 'X-Forwarded-Proto: https';

curl_setopt($ch, CURLOPT_CUSTOMREQUEST, $method);
curl_setopt($ch, CURLOPT_HTTPHEADER, $forwardHeaders);
curl_setopt($ch, CURLOPT_CONNECTTIMEOUT, CONNECT_TIMEOUT_SECONDS);
curl_setopt($ch, CURLOPT_TIMEOUT, EXEC_TIMEOUT_SECONDS);
curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
curl_setopt($ch, CURLOPT_HEADER, true);

// Forward request body for POST/PUT
if ($method === 'POST' || $method === 'PUT' || $method === 'PATCH') {
    $body = file_get_contents('php://input');
    if ($body !== false && $body !== '') {
        curl_setopt($ch, CURLOPT_POSTFIELDS, $body);
    }
}

$rawResponse = curl_exec($ch);

if ($rawResponse === false) {
    http_response_code(502);
    header('Content-Type: application/json; charset=utf-8');
    echo json_encode([
        'error' => 'Bad Gateway: Upstream request failed',
        'details' => curl_error($ch),
    ], JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT);
    curl_close($ch);
    exit;
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
        header($headerLine, false);
    }
}

header('Content-Length: ' . strlen($body));
echo $body;
