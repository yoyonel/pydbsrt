--[[
  vlsub_dbsrt.lua - VLSub (dbsrt Enhanced Edition)
  Based on original VLSub by Guillaume Le Maout (GPLv2)
  Strict 1:1 ISO VLSub ergonomics + "Search by pHash" addition
--]]

local app_title = "VLSub (dbsrt Enhanced)"
local app_version = "2.1.0"

function descriptor()
    return {
        title = app_title,
        version = app_version,
        author = "Lionel ATTY & Guillaume Le Maout",
        url = "https://github.com/yoyonel/pydbsrt",
        shortdesc = "VLSub ISO + Search by pHash",
        description = "<p>Standard VLSub interface with legacy hash, title/name search, "
            .. "and visual frame-exact perceptual hash (pHash) search.</p>",
        capabilities = { "menu", "input-listener" }
    }
end

-- Protocol, Network & UI Constants
local DEFAULT_HOST = "127.0.0.1"
local DEFAULT_DAEMON_PORT = 8765
local DEFAULT_HTTP_PORT = 80
local HTTP_BUFFER_CHUNK_SIZE = 8192
local CRLF_DOUBLE = "\r\n\r\n"
local CRLF_DOUBLE_LEN = 4
local MICROSECONDS_PER_SEC = 1000000
local MILLIS_PER_SEC = 1000.0
local MIN_SIGNIFICANT_DISPLAY_SHIFT_MS = 1.0
local DIALOG_SPACER_REPEAT = 100
local DEFAULT_OSD_DURATION_SEC = 3

-- Global state & configuration
local dlg = nil
local default_host = DEFAULT_HOST
local default_port = DEFAULT_DAEMON_PORT
local server_url = "http://" .. DEFAULT_HOST .. ":" .. DEFAULT_DAEMON_PORT
local osd_channel = nil

local file_info = {
    has_input = false,
    uri = nil,
    path = nil,
    filename = "",
    title = "",
    season = "",
    episode = ""
}

local candidates_store = {}
local last_matching_tier = nil
local last_matched_ref = nil
local input_table = {}
local selected_lang_code = "fre"

local languages = {
    { "fre", "French" },
    { "eng", "English" },
    { "all", "All" },
    { "spa", "Spanish" },
    { "ger", "German" },
    { "ita", "Italian" },
    { "por", "Portuguese" },
    { "dut", "Dutch" },
    { "pol", "Polish" },
    { "rus", "Russian" },
    { "ara", "Arabic" },
    { "chi", "Chinese" },
    { "jpn", "Japanese" },
    { "kor", "Korean" },
    { "cze", "Czech" },
    { "dan", "Danish" },
    { "fin", "Finnish" },
    { "ell", "Greek" },
    { "swe", "Swedish" },
    { "hun", "Hungarian" }
}

-- Mapping 3-letter ISO 639-2 codes to 2-letter ISO 639-1
local lang_map_2 = {
    fre = "fr",
    eng = "en",
    all = "all",
    spa = "es",
    ger = "de",
    ita = "it",
    por = "pt",
    dut = "nl",
    pol = "pl",
    rus = "ru",
    ara = "ar",
    chi = "zh",
    jpn = "ja",
    kor = "ko",
    cze = "cs",
    dan = "da",
    fin = "fi",
    ell = "el",
    swe = "sv",
    hun = "hu"
}

-- Help documentation HTML
local help_message = [[
<div style="font-family: sans-serif; font-size: 12px; line-height: 1.5; color: #222;">
  <h3>VLSub (dbsrt Enhanced) &mdash; Help &amp; Usage</h3>
  <p>Download subtitles from local dbsrt services and OpenSubtitles, and display them while watching a video.</p>
  <hr/>
  <h4><b>Usage:</b></h4>
  <p>1. Start your video in VLC.</p>
  <p>2. Select the desired subtitle language from the dropdown menu.</p>
  <p>3. Choose one of the <b>three available search methods</b>:</p>
  <ul>
    <li><b>Method 1: Search by hash</b><br/>
    Searches for subtitles using the standard 64-bit video file checksum (MovieHash). Recommended for exact file release matches.</li>
    <li><b>Method 2: Search by name</b><br/>
    Searches by video title, with optional Season and Episode numbers for TV shows.</li>
    <li><b>Method 3: Search by pHash</b> <i>(Powered by dbsrt engine)</i><br/>
    Performs visual perceptual matching of the active video stream (DCT-II SIMD) against certified reference prints in the dbsrt catalog. Enables automatic frame-exact resynchronization across different releases (web-dl, bluray, hdtv).</li>
  </ul>
  <hr/>
  <h4><b>Downloading Subtitles:</b></h4>
  <p>Select a subtitle candidate in the listbox and click <b>Download selection</b>. The subtitle will be downloaded, saved alongside your video file, and immediately attached to the running VLC player.</p>
  <p><i>Note: Subtitles are saved without asking confirmation, matching standard VLSub behavior.</i></p>
</div>
]]

-- Log Verbosity Levels: 0 = ERRORS, 1 = WARNINGS, 2 = INFO, 3 = DEBUG
local LOG_LEVEL_ERROR = 0
local LOG_LEVEL_WARN = 1
local LOG_LEVEL_INFO = 2
local LOG_LEVEL_DEBUG = 3

-- Active verbosity threshold (default: DEBUG for development & rich telemetry)
local active_log_level = LOG_LEVEL_DEBUG

-- Structured logging helpers
local function log_err(fmt, ...)
    if active_log_level >= LOG_LEVEL_ERROR then
        local msg = select("#", ...) > 0 and string.format(fmt, ...) or fmt
        if vlc and vlc.msg and vlc.msg.err then
            vlc.msg.err("[vlsub-dbsrt] [ERROR] " .. msg)
        elseif vlc and vlc.msg and vlc.msg.info then
            vlc.msg.info("[vlsub-dbsrt] [ERROR] " .. msg)
        end
    end
end

local function log_warn(fmt, ...)
    if active_log_level >= LOG_LEVEL_WARN then
        local msg = select("#", ...) > 0 and string.format(fmt, ...) or fmt
        if vlc and vlc.msg and vlc.msg.warn then
            vlc.msg.warn("[vlsub-dbsrt] [WARN] " .. msg)
        elseif vlc and vlc.msg and vlc.msg.info then
            vlc.msg.info("[vlsub-dbsrt] [WARN] " .. msg)
        end
    end
end

local function log_info(fmt, ...)
    if active_log_level >= LOG_LEVEL_INFO then
        local msg = select("#", ...) > 0 and string.format(fmt, ...) or fmt
        if vlc and vlc.msg and vlc.msg.info then
            vlc.msg.info("[vlsub-dbsrt] [INFO] " .. msg)
        end
    end
end

local function log_dbg(fmt, ...)
    if active_log_level >= LOG_LEVEL_DEBUG then
        local msg = select("#", ...) > 0 and string.format(fmt, ...) or fmt
        -- Route via vlc.msg.info with [DEBUG] tag because LibVLC drops vlc.msg.dbg
        -- unless VLC was specifically started with -vv from terminal
        if vlc and vlc.msg and vlc.msg.info then
            vlc.msg.info("[vlsub-dbsrt] [DEBUG] " .. msg)
        end
    end
end

-- OSD feedback
function notify_osd(msg, duration_sec, pos)
    duration_sec = duration_sec or DEFAULT_OSD_DURATION_SEC
    pos = pos or "top-left"
    if vlc.osd and vlc.osd.message then
        if not osd_channel and vlc.osd.channel_register then
            local ok, ch = pcall(function() return vlc.osd.channel_register() end)
            if ok and ch then osd_channel = ch end
        end
        pcall(function()
            vlc.osd.message(msg, osd_channel or 1, pos, duration_sec * MICROSECONDS_PER_SEC)
        end)
    end
    log_info("[OSD] %s", msg)
end

-- Server discovery via /tmp/dbsrt-server.json
function discover_server()
    local f = io.open("/tmp/dbsrt-server.json", "r")
    if not f then
        log_dbg("[Discovery] No /tmp/dbsrt-server.json found, keeping current endpoint: %s", server_url)
        return nil
    end
    local content = f:read("*all")
    f:close()

    local host = string.match(content, '"host":%s*"([^"]+)"')
    local port = string.match(content, '"port":%s*(%d+)')
    local url = string.match(content, '"url":%s*"([^"]+)"')

    if url and port then
        server_url = url
        default_host = host or DEFAULT_HOST
        default_port = tonumber(port) or DEFAULT_DAEMON_PORT
        log_info("[Discovery] Detected active daemon: %s (host=%s, port=%d)", server_url, default_host, default_port)
        return true
    end
    local preview = string.sub(content, 1, 80)
    log_warn("[Discovery] /tmp/dbsrt-server.json could not be parsed: %s", preview)
    return nil
end

function parse_url(url)
    local host, port = string.match(url, "http://([^:]+):?(%d*)")
    host = host or DEFAULT_HOST
    port = tonumber(port)
    if not port or port == 0 then port = DEFAULT_HTTP_PORT end
    return host, port
end

-- Strictly escape JSON string payload according to RFC 8259 (ERR-03 resilience)
function json_escape(str)
    if not str then return "" end
    local s = tostring(str)
    -- Order matters: escape backslash first
    s = s:gsub('\\', '\\\\')
    s = s:gsub('"', '\\"')
    s = s:gsub('\b', '\\b')
    s = s:gsub('\f', '\\f')
    s = s:gsub('\n', '\\n')
    s = s:gsub('\r', '\\r')
    s = s:gsub('\t', '\\t')
    return s
end

-- Strictly parse and unescape a JSON string field value supporting escaped quotes and backslashes (ERR-03)
function parse_json_string_field(json_str, key)
    if not json_str or not key then return nil end
    local pat = '"' .. key .. '"%s*:%s*"'
    local s, e = string.find(json_str, pat)
    if not s then return nil end
    local idx = e + 1
    local res = {}
    local len = string.len(json_str)
    while idx <= len do
        local c = string.sub(json_str, idx, idx)
        if c == '\\' then
            idx = idx + 1
            if idx <= len then
                local next_c = string.sub(json_str, idx, idx)
                if next_c == '"' then
                    table.insert(res, '"')
                elseif next_c == '\\' then
                    table.insert(res, '\\')
                elseif next_c == 'n' then
                    table.insert(res, '\n')
                elseif next_c == 'r' then
                    table.insert(res, '\r')
                elseif next_c == 't' then
                    table.insert(res, '\t')
                else
                    table.insert(res, next_c)
                end
            end
        elseif c == '"' then
            break
        else
            table.insert(res, c)
        end
        idx = idx + 1
    end
    return table.concat(res)
end

-- HTTP Client via TCP socket
function send_http_request(host, port, method, path, payload)
    local t0 = os.clock()
    local body_str = payload or ""
    log_info("[HTTP] [REQ] %s %s to %s:%d (body: %d bytes)", method, path, host, port, string.len(body_str))
    if string.len(body_str) > 0 then
        log_dbg("[HTTP] [REQ] Payload: %s", body_str)
    end

    local ok_conn, fd = pcall(function() return vlc.net.connect_tcp(host, port) end)
    if not ok_conn or not fd or fd < 0 then
        local elapsed_ms = (os.clock() - t0) * 1000.0
        log_err("[HTTP] Connection failed to %s:%d after %.2fms (is dbsrt-server running?)", host, port, elapsed_ms)
        return nil, string.format("Cannot connect to %s:%d", host, port)
    end
    log_dbg("[HTTP] Connected to %s:%d (socket fd=%d)", host, port, fd)

    local req = string.format(
        "%s %s HTTP/1.1\r\n"
        .. "Host: %s:%d\r\n"
        .. "Content-Type: application/json\r\n"
        .. "Content-Length: %d\r\n"
        .. "Connection: close\r\n\r\n%s",
        method, path, host, port, string.len(body_str), body_str
    )

    local ok_send, bytes_sent = pcall(function() return vlc.net.send(fd, req) end)
    if not ok_send or not bytes_sent or bytes_sent <= 0 then
        pcall(function() vlc.net.close(fd) end)
        local elapsed_ms = (os.clock() - t0) * 1000.0
        log_err("[HTTP] Failed to transmit bytes on socket fd=%s after %.2fms (send returned %s, daemon offline)",
            tostring(fd), elapsed_ms, tostring(bytes_sent))
        return nil, string.format("Cannot connect to %s:%d (daemon offline)", host, port)
    end
    log_dbg("[HTTP] Request transmitted (%d bytes), awaiting daemon response...", bytes_sent)

    local pollfds = {}
    pollfds[fd] = vlc.net.POLLIN
    local poll_status = vlc.net.poll(pollfds)
    if not poll_status or poll_status <= 0 or not pollfds[fd] or pollfds[fd] == 0 then
        pcall(function() vlc.net.close(fd) end)
        log_err("[HTTP] Socket poll error or hang up on fd=%s (poll status=%s, revents=%s)",
            tostring(fd), tostring(poll_status), tostring(pollfds[fd]))
        return nil, string.format("Connection dropped by %s:%d", host, port)
    end

    local chunk = vlc.net.recv(fd, HTTP_BUFFER_CHUNK_SIZE)
    local buf = ""
    local header_end = nil
    local content_length = nil
    local chunk_count = 0
    local total_bytes = 0

    while chunk and string.len(chunk) > 0 do
        chunk_count = chunk_count + 1
        total_bytes = total_bytes + string.len(chunk)
        buf = buf .. chunk
        if not header_end then
            header_end = string.find(buf, CRLF_DOUBLE)
            if header_end then
                local cl = string.match(buf, "[Cc]ontent%-[Ll]ength:%s*(%d+)")
                if cl then
                    content_length = tonumber(cl)
                    log_dbg("[HTTP] Parsed Content-Length header: %d bytes", content_length)
                end
            end
        end

        if header_end and content_length then
            local body_len = string.len(buf) - (header_end + CRLF_DOUBLE_LEN - 1)
            if body_len >= content_length then
                break
            end
        end

        -- Read next incoming chunk without blocking poll inside loop
        chunk = vlc.net.recv(fd, HTTP_BUFFER_CHUNK_SIZE)
    end

    pcall(function() vlc.net.close(fd) end)

    if not header_end then
        header_end = string.find(buf, CRLF_DOUBLE)
    end

    local elapsed_ms = (os.clock() - t0) * 1000.0
    if not header_end then
        log_err("[HTTP] Invalid HTTP response after %.2fms: %d bytes received, missing CRLF header boundary", elapsed_ms, total_bytes)
        return nil, "Invalid HTTP response from daemon"
    end

    local resp_body = string.sub(buf, header_end + CRLF_DOUBLE_LEN)
    log_info("[HTTP] [RESP] %s %s completed in %.2fms (%d bytes, %d chunk(s), body: %d bytes)",
        method, path, elapsed_ms, total_bytes, chunk_count, string.len(resp_body))
    local excerpt = string.sub(resp_body, 1, 150) .. (string.len(resp_body) > 150 and "..." or "")
    log_dbg("[HTTP] [RESP] Body excerpt: %s", excerpt)
    return resp_body, nil
end

-- Media file information extraction
function update_file_info()
    local item = vlc.input.item()
    if not item then
        file_info.has_input = false
        file_info.uri = nil
        file_info.path = nil
        file_info.filename = ""
        file_info.title = ""
        file_info.season = ""
        file_info.episode = ""
        log_dbg("[Media] No active input item playing")
        return false
    end

    file_info.has_input = true
    file_info.uri = item:uri()
    if not file_info.uri then
        file_info.path = nil
        log_warn("[Media] Active input item has empty URI")
        return false
    end

    file_info.path = vlc.strings.decode_uri(file_info.uri):gsub("^file://", "")
    file_info.filename = file_info.path:match("([^/\\]+)$") or file_info.path

    -- Strip extension from filename using last dot: ^(.+)%.([^%.]+)$
    local name = file_info.filename:match("^(.+)%.[^%.]+$") or file_info.filename
    name = name:gsub("[%.%_%+]", " ")

    local s, e = string.match(name, "[sS](%d%d?)[eE](%d%d?)")
    if not s then
        s, e = string.match(name, "(%d%d?)[xX](%d%d?)")
    end

    if s and e then
        file_info.season = tostring(tonumber(s))
        file_info.episode = tostring(tonumber(e))
        name = string.gsub(name, "[sS]%d%d?[eE]%d%d?.*", "")
        name = string.gsub(name, "%d%d?[xX]%d%d?.*", "")
    else
        file_info.season = ""
        file_info.episode = ""
    end

    name = string.gsub(name, "%b()", "")
    name = string.gsub(name, "%b[]", "")
    name = string.gsub(name, "%s+", " ")
    name = string.gsub(name, "^%s*(.-)%s*$", "%1")

    file_info.title = name
    log_info("[Media] Active file: '%s' (title='%s', S%sE%s, path='%s')",
        file_info.filename, file_info.title, file_info.season ~= "" and file_info.season or "?",
        file_info.episode ~= "" and file_info.episode or "?", file_info.path)
    return true
end

function get_selected_lang()
    if not input_table["language"] then return "fr" end
    local val = input_table["language"]:get_value()
    if type(val) == "number" and languages[val] then
        local code3 = languages[val][1]
        return lang_map_2[code3] or "fr"
    end
    return "fr"
end

function setMessage(str)
    if input_table["message"] then
        input_table["message"]:set_text(str or "")
        if dlg and dlg.update then dlg:update() end
    end
end

-- Main dialog creation (100% ISO VLSub layout with Search by pHash added)
function interface_main()
    -- Row 1: Subtitles language dropdown + Search by hash button
    dlg:add_label("Subtitles language:", 1, 1, 1, 1)
    input_table["language"] = dlg:add_dropdown(2, 1, 2, 1)
    for idx, lang in ipairs(languages) do
        input_table["language"]:add_value(lang[2], idx)
    end
    dlg:add_button("Search by hash", search_by_hash, 4, 1, 1, 1)

    -- Row 2: Title input + Search by name button
    dlg:add_label("Title:", 1, 2, 1, 1)
    input_table["title"] = dlg:add_text_input(file_info.title or "", 2, 2, 2, 1)
    dlg:add_button("Search by name", search_by_name, 4, 2, 1, 1)

    -- Row 3: Season input + Search by pHash button (3rd choice)
    dlg:add_label("Season (series):", 1, 3, 1, 1)
    input_table["seasonNumber"] = dlg:add_text_input(file_info.season or "", 2, 3, 2, 1)
    dlg:add_button("Search by pHash", search_by_phash, 4, 3, 1, 1)

    -- Row 4: Episode input
    dlg:add_label("Episode (series):", 1, 4, 1, 1)
    input_table["episodeNumber"] = dlg:add_text_input(file_info.episode or "", 2, 4, 2, 1)

    -- Row 5: Results List (Full width across all 4 columns)
    input_table["mainlist"] = dlg:add_list(1, 5, 4, 1)

    -- Row 6: Message Label (Status feedback)
    input_table["message"] = dlg:add_label(" ", 1, 6, 4, 1)

    -- Row 7: Action Toolbar (Show help, Show config, Download selection, Close)
    dlg:add_button("Show help", show_help, 1, 7, 1, 1)
    dlg:add_button("Show config", show_conf, 2, 7, 1, 1)
    dlg:add_button("Download selection", download_selection, 3, 7, 1, 1)
    dlg:add_button("Close", close, 4, 7, 1, 1)

    display_subtitles()
end

-- Configuration dialog
function interface_config()
    dlg:add_label("<b>VLSub Configuration:</b>", 1, 1, 4, 1)

    dlg:add_label("Default language:", 1, 2, 2, 1)
    input_table["conf_lang"] = dlg:add_dropdown(3, 2, 2, 1)
    for idx, lang in ipairs(languages) do
        input_table["conf_lang"]:add_value(lang[2], idx)
    end

    dlg:add_label("dbsrt Server Endpoint:", 1, 3, 2, 1)
    input_table["conf_server"] = dlg:add_text_input(server_url, 3, 3, 2, 1)

    input_table["conf_status"] = dlg:add_label(" ", 1, 4, 4, 1)

    dlg:add_button("Cancel", show_main, 3, 5, 1, 1)
    dlg:add_button("Save", save_config, 4, 5, 1, 1)
end

function save_config()
    if input_table["conf_server"] then
        local custom_url = input_table["conf_server"]:get_text()
        if custom_url and custom_url ~= "" then
            server_url = custom_url
        end
    end
    show_main()
end

-- Help dialog
function interface_help()
    input_table["help"] = dlg:add_html(help_message, 1, 1, 4, 1)
    dlg:add_label(string.rep("&nbsp;", DIALOG_SPACER_REPEAT), 1, 2, 3, 1)
    dlg:add_button("OK", show_main, 4, 2, 1, 1)
end

-- Menu and window lifecycle
function trigger_menu(dlg_id)
    close_dlg()
    if dlg_id == 1 then
        dlg = vlc.dialog(app_title)
        interface_main()
    elseif dlg_id == 2 then
        dlg = vlc.dialog(app_title .. " - Configuration")
        interface_config()
    elseif dlg_id == 3 then
        dlg = vlc.dialog(app_title .. " - Help")
        interface_help()
    end
    collectgarbage()
end

function show_main()
    trigger_menu(1)
end

function show_conf()
    trigger_menu(2)
end

function show_help()
    trigger_menu(3)
end

function close_dlg()
    if dlg ~= nil then
        dlg:hide()
    end
    dlg = nil
    for k in pairs(input_table) do
        input_table[k] = nil
    end
    collectgarbage()
end

function close()
    vlc.deactivate()
end

function activate()
    log_info("==================================================")
    log_info("=== VLSub (dbsrt Enhanced) v%s Initialized ===", app_version)
    log_info("==================================================")
    discover_server()
    update_file_info()
    show_main()
end

function deactivate()
    log_info("=== VLSub (dbsrt Enhanced) Deactivated ===")
    if dlg then
        dlg:hide()
    end
    dlg = nil
    for k in pairs(input_table) do
        input_table[k] = nil
    end
end

function input_changed()
    update_file_info()
    if dlg and input_table["title"] and type(input_table["title"]) == "userdata" then
        input_table["title"]:set_text(file_info.title or "")
        input_table["seasonNumber"]:set_text(file_info.season or "")
        input_table["episodeNumber"]:set_text(file_info.episode or "")
    end
end

function meta_changed()
    return false
end

-- Unified search runner via dbsrt-server REST API
function run_search(mode)
    local t_start = os.clock()
    log_info("=== [Search] Triggered method: %s ===", mode:upper())

    if not file_info.has_input or not file_info.path then
        log_warn("[Search] Aborted: No active media file playing in VLC")
        setMessage("<b>Error:</b> No active media playing in VLC.")
        return
    end

    discover_server()
    local host, port = parse_url(server_url)
    local lang = get_selected_lang()
    log_info("[Search] Connecting to daemon at %s:%d | Requested language: '%s'", host, port, lang)

    local title_query = ""
    local season_query = nil
    local episode_query = nil

    if input_table["title"] then
        title_query = input_table["title"]:get_text() or ""
    end
    if input_table["seasonNumber"] then
        season_query = tonumber(input_table["seasonNumber"]:get_text())
    end
    if input_table["episodeNumber"] then
        episode_query = tonumber(input_table["episodeNumber"]:get_text())
    end

    log_info("[Search] Query parameters: title='%s', season=%s, episode=%s, media='%s'",
        title_query, tostring(season_query), tostring(episode_query), file_info.filename)

    setMessage(string.format("<b>Searching subtitles (%s)...</b>", mode:upper()))

    local payload = string.format(
        '{"media_path":"%s","search_mode":"%s","language":"%s","title":"%s"',
        json_escape(file_info.path),
        json_escape(mode),
        json_escape(lang),
        json_escape(title_query or "")
    )
    if season_query then
        payload = payload .. string.format(',"season":%d', season_query)
    end
    if episode_query then
        payload = payload .. string.format(',"episode":%d', episode_query)
    end
    payload = payload .. "}"

    local t_req_start = os.clock()
    local body, err = send_http_request(host, port, "POST", "/v1/search", payload)
    local req_elapsed_ms = (os.clock() - t_req_start) * 1000.0

    if not body then
        log_err("[Search] Daemon unreachable after %.2fms: %s", req_elapsed_ms, err or "offline")
        setMessage(string.format("<b>Error:</b> Cannot connect to dbsrt-server (%s)", err or "offline"))
        return
    end

    local matched_media = string.match(body, '"matched_media":%s*"([^"]+)"')
    local shift_frames = string.match(body, '"shift_frames":%s*([%d%-]+)')
    local shift_ms_str = string.match(body, '"shift_ms":%s*([%d%.%-]+)')
    local confidence_str = string.match(body, '"confidence_percent":%s*([%d%.]+)')
    local matching_tier = string.match(body, '"matching_tier":%s*"([^"]+)"')

    if matching_tier and string.find(matching_tier, "Remote Relay") then
        log_info("[Relay] ✓ Media identified via Remote Server (%s)", matching_tier)
    elseif matching_tier and matching_tier ~= "" then
        log_dbg("[Search] Engine tier: %s", matching_tier)
    end

    local is_relay = matching_tier and (string.find(matching_tier, "Remote Relay") ~= nil)
    last_matching_tier = matching_tier
    last_matched_ref = matched_media

    local origin_banner = is_relay and "CLOUD o2switch relay" or "LOCAL DAEMON"
    log_info("================================================================================")
    log_info("[MATCH IDENTIFIER] SOURCE: %s | ENGINE: %s", origin_banner, matching_tier or "Local MIH")
    log_info("[MATCH IDENTIFIER] Reference: '%s' | Confidence: %s%% | Network RTT: %.2fms",
        matched_media or "none", confidence_str or "0.0", req_elapsed_ms)
    log_info("================================================================================")

    log_info("[Search] Server response in %.2fms: matched_ref='%s', confidence=%s%%, visual shift=%sframes / %sms",
        req_elapsed_ms, matched_media or "none", confidence_str or "0.0", shift_frames or "0", shift_ms_str or "0.0")

    -- Extract & log fingerprints (MovieHash & pHash frames) in DEBUG mode
    local moviehash = string.match(body, '"moviehash":%s*"([^"]+)"')
    if moviehash then
        log_dbg("[Fingerprint] 64-bit MovieHash (OpenSubtitles): %s", moviehash)
    end

    local fp_start = string.find(body, '"fingerprint_preview":%s*%[')
    if fp_start then
        local fp_end = string.find(body, "%]", fp_start)
        if fp_end then
            local fp_str = string.sub(body, fp_start, fp_end)
            local fp_hexes = {}
            for h in string.gmatch(fp_str, '"(0x[0-9a-fA-F]+)"') do
                table.insert(fp_hexes, h)
            end
            if #fp_hexes > 0 then
                log_dbg("[Fingerprint] Server-computed visual pHash preview (%d frames): %s",
                    #fp_hexes, table.concat(fp_hexes, ", "))
            end
        end
    end

    for k in pairs(candidates_store) do candidates_store[k] = nil end
    local cand_start = string.find(body, '"candidates":%s*%[')
    if cand_start then
        local search_from = string.find(body, "%[", cand_start) + 1
        local cand_end = string.find(body, "%]%s*,%s*\"timing\"", search_from)
        if not cand_end then
            cand_end = string.find(body, "%]%s*}", search_from)
        end
        local cand_block = cand_end and string.sub(body, search_from, cand_end - 1) or string.sub(body, search_from)
        for item in string.gmatch(cand_block, "{([^}]+)}") do
            local c_title = string.match(item, '"title":%s*"([^"]+)"') or "Subtitle"
            local c_lang = string.match(item, '"language":%s*"([^"]+)"') or lang
            local c_source = string.match(item, '"source":%s*"([^"]+)"') or "unknown"
            local c_id = string.match(item, '"candidate_id":%s*"([^"]+)"') or ""
            local c_fps = string.match(item, '"fps":%s*([%d%.]+)') or ""
            local c_score = tonumber(string.match(item, '"score":%s*([%d%.]+)')) or 1.0
            local c_shift = tonumber(string.match(item, '"shift_ms":%s*([%d%.%-]+)'))

            table.insert(candidates_store, {
                title = c_title,
                language = c_lang,
                source = c_source,
                candidate_id = c_id,
                fps = c_fps,
                score = c_score,
                shift_ms = c_shift
            })

            local shift_desc = c_shift and string.format("%+.2fms", c_shift) or "0.0ms"
            log_dbg("[Search] Candidate #%d: '%s' [lang=%s, source=%s, shift=%s, id=%s]",
                #candidates_store, c_title, c_lang:upper(), c_source, shift_desc, c_id)
        end
    end

    display_subtitles()

    local total_elapsed_ms = (os.clock() - t_start) * 1000.0
    local origin_short = is_relay and "[CLOUD o2switch]" or "[LOCAL]"
    if #candidates_store == 0 then
        log_info("[Search] Complete %s in %.2fms: 0 subtitles found", origin_short, total_elapsed_ms)
        setMessage(string.format("<b>%s Complete:</b> No subtitles found.", origin_short))
    else
        local ref_label = matched_media or (title_query ~= "" and title_query) or "ok"
        log_info("[Search] Complete %s in %.2fms: %d candidate(s) loaded into UI listbox (ref: %s)", origin_short, total_elapsed_ms, #candidates_store, ref_label)
        setMessage(string.format("<b>%s MATCH:</b> %d subtitle(s) found (ref: %s, %.0fms).",
            origin_short, #candidates_store, ref_label, req_elapsed_ms))
    end
end

function search_by_phash()
    run_search("phash")
end

function search_by_hash()
    run_search("hash")
end

function search_by_name()
    run_search("name")
end

-- Format and populate the VLSub standard listbox
function display_subtitles()
    if not input_table["mainlist"] then return end
    input_table["mainlist"]:clear()

    if #candidates_store == 0 then
        return
    end

    local is_relay = last_matching_tier and (string.find(last_matching_tier, "Remote Relay") ~= nil)
    for idx, c in ipairs(candidates_store) do
        local source_tag = c.source:upper()
        if c.source == "certified" or c.source == "certified_synced" then
            source_tag = is_relay and "[CLOUD o2switch] pHash certified" or "pHash certified"
        elseif c.source == "container" then
            source_tag = "container"
        elseif c.source == "catalog" then
            source_tag = is_relay and "[CLOUD o2switch] catalog" or "catalog"
        elseif c.source == "remote" then
            source_tag = "OpenSubtitles"
        end

        local display_text = string.format("%s [%s] (%s)", c.title, c.language:upper(), source_tag)
        input_table["mainlist"]:add_value(display_text, idx)
    end
end

-- Download and apply user selected subtitle
function download_selection()
    local t_start = os.clock()
    if not input_table["mainlist"] then return end
    local selection = input_table["mainlist"]:get_selection()
    local sel_idx = nil
    for idx, _ in pairs(selection) do
        sel_idx = idx
        break
    end

    if not sel_idx or not candidates_store[sel_idx] then
        log_warn("[Download] Aborted: No subtitle selected in listbox by user")
        setMessage("<b>No subtitle selected:</b> Please select a subtitle in the list first.")
        return
    end

    local candidate = candidates_store[sel_idx]
    log_info("=== [Download] User selected entry #%d: '%s' [lang=%s, source=%s] ===",
        sel_idx, candidate.title, candidate.language:upper(), candidate.source)

    setMessage(string.format("<b>Downloading:</b> %s [%s]...", candidate.title, candidate.language:upper()))

    local is_resync = false
    local shift_param = "null"
    if candidate.shift_ms and math.abs(candidate.shift_ms) > 1.0 then
        is_resync = true
        shift_param = string.format("%.2f", candidate.shift_ms)
        log_info("[Download] Visual resynchronization active: shift_ms=%+.2fms (%.2fs)",
            candidate.shift_ms, candidate.shift_ms / MILLIS_PER_SEC)
    else
        log_info("[Download] Standard 1:1 injection without shift (shift_ms=0.0)")
    end

    discover_server()
    local host, port = parse_url(server_url)
    local payload = string.format(
        '{"media_path":"%s","source":"%s","candidate_id":"%s","language":"%s","resync":%s,"shift_ms":%s}',
        json_escape(file_info.path),
        json_escape(candidate.source),
        json_escape(candidate.candidate_id or ""),
        json_escape(candidate.language),
        tostring(is_resync),
        shift_param
    )

    log_info("[Download] Requesting download from daemon at %s:%d (candidate_id='%s')",
        host, port, candidate.candidate_id or "")

    local t_req = os.clock()
    local body, err = send_http_request(host, port, "POST", "/v1/download", payload)
    local req_ms = (os.clock() - t_req) * 1000.0

    if not body then
        log_err("[Download] Download failed after %.2fms: %s", req_ms, err or "No response from dbsrt-server")
        setMessage(string.format("<b>Download error:</b> %s", err or "No response from dbsrt-server"))
        notify_osd("dbsrt: Download failed", DEFAULT_OSD_DURATION_SEC)
        return
    end

    local success = string.match(body, '"success":%s*(%a+)')
    local out_srt = parse_json_string_field(body, "output_srt_path") or string.match(body, '"output_srt_path":%s*"([^"]+)"')
    local shift_ms = tonumber(string.match(body, '"shift_ms":%s*([%d%.%-]+)')) or 0.0

    if success == "true" and out_srt and out_srt ~= "" then
        log_info("[Download] Server saved subtitle in %.2fms -> '%s' (applied shift: %+.2fms)",
            req_ms, out_srt, shift_ms)
        log_info("[Download] Attaching subtitle to VLC stream: vlc.input.add_subtitle('%s', autoselect=true)", out_srt)

        local ok_sub, sub_err = pcall(function() return vlc.input.add_subtitle(out_srt, true) end)
        if ok_sub then
            log_info("[Download] ✓ Subtitle attached successfully to active playback")
        else
            log_err("[Download] vlc.input.add_subtitle failed: %s", tostring(sub_err))
        end

        local is_relay = last_matching_tier and (string.find(last_matching_tier, "Remote Relay") ~= nil)
        local origin_short = is_relay and "[CLOUD o2switch]" or "[LOCAL]"
        local out_basename = out_srt:match("([^/\\]+)$") or out_srt
        local shift_desc = ""
        if math.abs(shift_ms) > MIN_SIGNIFICANT_DISPLAY_SHIFT_MS then
            shift_desc = string.format(" (shift: %+.2fs)", shift_ms / MILLIS_PER_SEC)
        end
        setMessage(string.format("<b>%s Subtitles loaded:</b> %s%s", origin_short, out_basename, shift_desc))
        notify_osd(string.format("✓ %s Subtitles loaded: %s", origin_short, out_basename), DEFAULT_OSD_DURATION_SEC)

        local total_dl_ms = (os.clock() - t_start) * 1000.0
        log_info("=== [Download] %s Complete in %.2fms: loaded '%s'%s ===", origin_short, total_dl_ms, out_basename, shift_desc)
    else
        local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "Failed to download subtitle."
        log_err("[Download] Server download failed: %s", err_msg)
        setMessage(string.format("<b>Download failed:</b> %s", err_msg))
    end
end

-- Export internal state for unit test harness
if _G and _G._VLSub_test_hook then
    _G._VLSub_state = {
        file_info = file_info,
        candidates_store = candidates_store,
        input_table = input_table,
        parse_url = parse_url,
        update_file_info = update_file_info,
        send_http_request = send_http_request,
        run_search = run_search,
        search_by_phash = search_by_phash,
        search_by_hash = search_by_hash,
        search_by_name = search_by_name,
        download_selection = download_selection,
        show_help = show_help,
        json_escape = json_escape,
        parse_json_string_field = parse_json_string_field,
        display_subtitles = display_subtitles
    }
end
