<?php
/**
 * Ultra-lightweight resilient reverse proxy gateway with on-demand auto-spawn
 * and API Key authentication for dbsrt-server on o2switch cPanel Cloud.
 *
 * Relays public HTTPS requests from Apache/LiteSpeed to the local Rust dbsrt-server daemon
 * listening on 127.0.0.1:8765 without requiring Apache mod_proxy privileges.
 * Automatically spawns the daemon if inactive or harvested by CloudLinux.
 * Enforces strict API Key authentication (X-API-Key or Bearer token); fails closed without a key file.
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
const SPAWN_LOCK_REL_PATH = '/dbsrt/run/gateway_spawn.lock';

// Derive user home directory relative to public_html/dbsrt
$homeDir = dirname(__DIR__, 2);
$secretKeyFile = $homeDir . SECRET_KEY_REL_PATH;

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

function is_authenticated(array $validKeys): bool
{
    // Extract provided client token from headers
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

function is_daemon_healthy(int $connectTimeoutMs, int $timeoutMs): bool
{
    $ch = curl_init(sprintf('%s:%d/v1/health', UPSTREAM_HOST, UPSTREAM_PORT));
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
    $lock = fopen($homeDir . SPAWN_LOCK_REL_PATH, 'c');
    if ($lock === false || !flock($lock, LOCK_EX)) {
        return false;
    }

    try {
        // Another request may have spawned it while we waited for the lock
        if (is_daemon_healthy(1500, 2000)) {
            return true;
        }

        $bin = escapeshellarg($homeDir . '/dbsrt/bin/dbsrt-server');
        $catalog = escapeshellarg($homeDir . '/dbsrt/data/catalog.dbphash');
        $srtDir = escapeshellarg($homeDir . '/dbsrt/data/SRT');
        // Loose .phash are packed into the catalog by o2switch_ctl.sh start (LVE memory cap)
        $phashDir = escapeshellarg($homeDir . '/dbsrt/run/empty_phash');
        $log = escapeshellarg($homeDir . '/dbsrt/logs/dbsrt-server.log');

        exec(sprintf(
            'setsid %s --host 127.0.0.1 --port %d --dbphash %s --subtitles-dir %s --phash-dir %s </dev/null >> %s 2>&1 &',
            $bin,
            UPSTREAM_PORT,
            $catalog,
            $srtDir,
            $phashDir,
            $log
        ));

        // Poll until healthy or retries exhausted
        for ($i = 0; $i < SPAWN_MAX_RETRIES; $i++) {
            usleep(SPAWN_RETRY_INTERVAL_MICROSECONDS);
            if (is_daemon_healthy(1000, 1500)) {
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

// 1. Strict Authentication Check (fail closed when no key is provisioned)
$validKeys = load_valid_keys($secretKeyFile);
if ($validKeys === []) {
    send_json_error(503, ['error' => 'Service Unavailable: gateway API key not provisioned']);
}
if (!is_authenticated($validKeys)) {
    send_json_error(401, [
        'error' => 'Unauthorized',
        'message' => 'Valid API key required via X-API-Key header or Bearer token.',
    ]);
}

// 2. Determine requested sub-path
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

$upstreamQuery = $_SERVER['QUERY_STRING'] ?? '';
$upstreamUrl = sprintf('%s:%d%s%s', UPSTREAM_HOST, UPSTREAM_PORT, $path, $upstreamQuery !== '' ? '?' . $upstreamQuery : '');
$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';

// Forward request headers, minus hop-by-hop, credentials and client-supplied forwarding headers
$droppedHeaders = ['host', 'content-length', 'connection', 'x-api-key', 'authorization', 'x-forwarded-for', 'x-forwarded-proto'];
$forwardHeaders = [];
foreach (getallheaders() as $name => $value) {
    if (!in_array(strtolower($name), $droppedHeaders, true)) {
        $forwardHeaders[] = sprintf('%s: %s', $name, $value);
    }
}
$forwardHeaders[] = sprintf('X-Forwarded-For: %s', $_SERVER['REMOTE_ADDR'] ?? '127.0.0.1');
$forwardHeaders[] = 'X-Forwarded-Proto: https';

$requestBody = in_array($method, ['POST', 'PUT', 'PATCH'], true) ? file_get_contents('php://input') : false;

$ch = curl_init($upstreamUrl);
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

// 3. Lazy on-demand activation: spawn only when the daemon refused the connection (nothing sent yet)
if ($rawResponse === false && curl_errno($ch) === CURLE_COULDNT_CONNECT) {
    if (!spawn_daemon($homeDir)) {
        curl_close($ch);
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
        header($headerLine, false);
    }
}

header('Content-Length: ' . strlen($body));
echo $body;
