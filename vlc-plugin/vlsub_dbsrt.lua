--[[
  vlsub_dbsrt.lua - VLSub (dbsrt Studio Edition)
  Based on VLSub by Guillaume Le Maout (GPLv2)
  Enhanced by Lionel ATTY for pydbsrt (Rust Axum Daemon, Multi-providers & pHash Sync)
--]]

function descriptor()
    return {
        title = "VLSub (dbsrt Enhanced)",
        version = "2.0.0",
        author = "Lionel ATTY & Guillaume Le Maout",
        url = "https://github.com/yoyonel/pydbsrt",
        shortdesc = "VLSub Studio Edition: REST API & pHash Frame-Exact Sync",
        description = "<div style=\"background-color: #0f172a; color: #f8fafc; padding: 12px;\">"
            .. "<h3>VLSub — dbsrt Studio Edition (v2.0)</h3>"
            .. "<p>Modernized VLSub fork powered by the local Rust Axum daemon, "
            .. "certified relational couplings (PostgreSQL 17 + Zero-DB), local catalog, "
            .. "OpenSubtitles REST fallback, and frame-exact perceptual hash (pHash) synchronization.</p>"
            .. "</div>",
        capabilities = { "input-listener" }
    }
end

-- Global state & configuration
local dlg = nil
local default_host = "127.0.0.1"
local default_port = 8765
local server_url = "http://127.0.0.1:8765"
local osd_channel = nil

local daemon_state = {
    online = false,
    version = "?",
    uptime = "0",
    phash_count = "0",
    certified_count = "0"
}

local file_info = {
    has_input = false,
    uri = nil,
    path = nil,
    filename = "",
    clean_name = "",
    title = "",
    season = "",
    episode = "",
}

local candidates_store = {}
local input_table = {}
local last_downloaded_srt = nil
local last_downloaded_lang = nil

local languages = {
    { "fr", "French — Francais [FR]      " },
    { "en", "English — International [EN]" },
    { "all", "All Languages — Auto [ALL]  " },
    { "es", "Spanish — Espanol [ES]      " },
    { "de", "German — Deutsch [DE]       " },
    { "it", "Italian — Italiano [IT]     " },
    { "pt", "Portuguese — Portugues [PT] " },
    { "ru", "Russian — Russkiy [RU]      " },
    { "ja", "Japanese — Nihongo [JA]     " },
    { "zh", "Chinese — Zhongwen [ZH]     " }
}

-- RichText HTML Console & Banner Builders (Dark Slate #0f172a for high contrast on Qt dialogs)
function render_dark_card(title_html, body_html, border_color)
    local accent = border_color or "#38bdf8"
    return string.format(
        "<table width=\"100%%\" cellspacing=\"0\" cellpadding=\"8\" bgcolor=\"#0f172a\" style=\"border-left: 4px solid %s;\">"
        .. "<tr><td>"
        .. "<div style=\"font-family: monospace; font-size: 11px; color: #f8fafc;\">%s</div>"
        .. "<div style=\"font-family: sans-serif; font-size: 11px; color: #cbd5e1; margin-top: 3px;\">%s</div>"
        .. "</td></tr></table>",
        accent, title_html, body_html
    )
end

function update_header_banner()
    if not input_table["header_banner"] then
        return
    end

    local status_badge = ""
    if daemon_state.online then
        status_badge = string.format(
            "<font color=\"#34d399\"><b>[ONLINE]</b></font> <font color=\"#94a3b8\">%s &bull; v%s &bull; %s .phash indexed</font>",
            server_url, daemon_state.version, daemon_state.phash_count
        )
    else
        status_badge = string.format(
            "<font color=\"#fb7185\"><b>[OFFLINE]</b></font> <font color=\"#94a3b8\">%s (run 'task stack:up')</font>",
            server_url
        )
    end

    local title_line = "<font color=\"#38bdf8\"><b>DBSRT STUDIO EXTENSION</b></font> &nbsp;|&nbsp; " .. status_badge

    local media_line = ""
    if file_info.has_input and file_info.filename ~= "" then
        local se_info = ""
        if file_info.season ~= "" and file_info.episode ~= "" then
            se_info = string.format(" &nbsp;|&nbsp; <font color=\"#38bdf8\">S%02dE%02d</font>",
                tonumber(file_info.season) or 0, tonumber(file_info.episode) or 0)
        end
        media_line = string.format(
            "<font color=\"#94a3b8\">Active Media:</font> <font color=\"#f8fafc\"><b>%s</b></font>%s",
            file_info.filename, se_info
        )
    else
        media_line = "<font color=\"#fbbf24\">No active video stream detected in VLC.</font>"
    end

    input_table["header_banner"]:set_text(render_dark_card(title_line, media_line, "#38bdf8"))
end

function set_console_message(title_html, body_html, accent_color)
    if input_table["message"] then
        input_table["message"]:set_text(render_dark_card(title_html, body_html, accent_color or "#38bdf8"))
    end
end

-- OSD feedback (top-left to avoid overlapping VLC native top-right track notification)
function notify_osd(msg, duration_sec, pos)
    duration_sec = duration_sec or 3
    pos = pos or "top-left"
    if vlc.osd and vlc.osd.message then
        if not osd_channel and vlc.osd.channel_register then
            local ok, ch = pcall(function() return vlc.osd.channel_register() end)
            if ok and ch then osd_channel = ch end
        end
        pcall(function()
            vlc.osd.message(msg, osd_channel or 1, pos, duration_sec * 1000000)
        end)
    end
    if vlc.msg and vlc.msg.info then
        vlc.msg.info("[vlsub-dbsrt] " .. msg)
    end
end

-- Server discovery
function discover_server()
    local f = io.open("/tmp/dbsrt-server.json", "r")
    if not f then
        return nil
    end
    local content = f:read("*all")
    f:close()

    local host = string.match(content, '"host":%s*"([^"]+)"')
    local port = string.match(content, '"port":%s*(%d+)')
    local url = string.match(content, '"url":%s*"([^"]+)"')

    if url and port then
        server_url = url
        default_host = host or "127.0.0.1"
        default_port = tonumber(port) or 8085
        return true
    end
    return nil
end

function parse_url(url)
    local host, port = string.match(url, "http://([^:]+):?(%d*)")
    host = host or "127.0.0.1"
    port = tonumber(port)
    if not port or port == 0 then
        port = 80
    end
    return host, port
end

-- HTTP Client via TCP socket
function send_http_request(host, port, method, path, payload)
    local ok_conn, fd = pcall(function() return vlc.net.connect_tcp(host, port) end)
    if not ok_conn or not fd or fd < 0 then
        return nil, string.format("Cannot connect to %s:%d", host, port)
    end

    local body_str = payload or ""
    local req = string.format(
        "%s %s HTTP/1.1\r\n"
        .. "Host: %s:%d\r\n"
        .. "Content-Type: application/json\r\n"
        .. "Content-Length: %d\r\n"
        .. "Connection: close\r\n\r\n%s",
        method, path, host, port, string.len(body_str), body_str
    )

    local ok_send = pcall(function() vlc.net.send(fd, req) end)
    if not ok_send then
        pcall(function() vlc.net.close(fd) end)
        return nil, "Failed to send HTTP request"
    end

    local resp = ""
    local pollfds = { [fd] = vlc.net.POLLIN }

    for _ = 1, 60 do
        local ok_poll, p = pcall(function() return vlc.net.poll(pollfds) end)
        if not ok_poll then break end
        if p and p > 0 then
            local ok_recv, chunk = pcall(function() return vlc.net.recv(fd, 4096) end)
            if ok_recv and chunk and string.len(chunk) > 0 then
                resp = resp .. chunk
                local header_end = string.find(resp, "\r\n\r\n")
                if header_end then
                    local cl = string.match(resp, "[Cc]ontent%-[Ll]ength:%s*(%d+)")
                    if cl then
                        local body_len = string.len(resp) - (header_end + 3)
                        if body_len >= tonumber(cl) then
                            break
                        end
                    end
                end
            else
                break
            end
        end
        vlc.misc.mwait(vlc.misc.mdate() + 10000) -- 10ms
    end

    pcall(function() vlc.net.close(fd) end)

    local header_end = string.find(resp, "\r\n\r\n")
    if not header_end then
        return nil, "Invalid HTTP response from daemon"
    end
    return string.sub(resp, header_end + 4), nil
end

-- Media file information extraction
function update_file_info()
    local item = vlc.input.item()
    if not item then
        file_info.has_input = false
        file_info.uri = nil
        file_info.path = nil
        file_info.filename = ""
        file_info.clean_name = ""
        file_info.title = ""
        file_info.season = ""
        file_info.episode = ""
        return false
    end

    file_info.has_input = true
    file_info.uri = item:uri()
    local path = vlc.strings.decode_uri(file_info.uri)
    path = path:gsub("^file://", "")
    file_info.path = path

    local fname = string.match(path, "([^/]+)$") or path
    file_info.filename = fname
    local stem = fname:gsub("%.%w+$", "")
    file_info.clean_name = stem:gsub("[%._]", " ")

    local show, s, e = string.match(file_info.clean_name, "(.+)[sS](%d?%d)[eE](%d%d)")
    if not show then
        show, s, e = string.match(file_info.clean_name, "(.-)(%d?%d)[xX](%d%d)")
    end

    if show then
        file_info.title = show:gsub("^%s*(.-)%s*$", "%1")
        file_info.season = tostring(tonumber(s) or "")
        file_info.episode = tostring(tonumber(e) or "")
    else
        file_info.title = file_info.clean_name
        file_info.season = ""
        file_info.episode = ""
    end

    return true
end

-- VLC Lifecycle Callbacks
function activate()
    local ok, err = pcall(function()
        discover_server()
        update_file_info()
        create_dialog()
        check_daemon_status()
    end)
    if not ok then
        vlc.msg.err("[vlsub-dbsrt] Error in activate: " .. tostring(err))
    end
end

function deactivate()
    if dlg then
        dlg:delete()
        dlg = nil
    end
end

function close()
    vlc.deactivate()
end

function input_changed()
    update_file_info()
    if dlg and input_table["title"] then
        input_table["title"]:set_text(file_info.title or "")
        input_table["season"]:set_text(file_info.season or "")
        input_table["episode"]:set_text(file_info.episode or "")
        update_header_banner()
    end
end

-- Modernized Studio Dialog Layout (5-column Grid, High-Contrast Dark Telemetry Cards)
function create_dialog()
    dlg = vlc.dialog("VLSub (dbsrt Enhanced)")

    -- Row 1: Studio Slate Header Banner (Daemon Telemetry + Active Media)
    input_table["header_banner"] = dlg:add_label("", 1, 1, 5, 1)
    update_header_banner()

    -- Row 2: Language Selector & Primary Search Modes
    dlg:add_label("<b>Language:</b>", 1, 2, 1, 1)
    input_table["language"] = dlg:add_dropdown(2, 2, 1, 1)
    for idx, lang in ipairs(languages) do
        input_table["language"]:add_value(lang[2], idx)
    end

    dlg:add_button("Smart Search (All)", function() do_search("all") end, 3, 2, 1, 1)
    dlg:add_button("Search by pHash", function() do_search("phash") end, 4, 2, 1, 1)
    dlg:add_button("Search by Hash", function() do_search("hash") end, 5, 2, 1, 1)

    -- Row 3: Release Metadata (Title + Season / Episode) & Name Search
    dlg:add_label("<b>Title / S-E:</b>", 1, 3, 1, 1)
    input_table["title"] = dlg:add_text_input(file_info.title or "", 2, 3, 1, 1)
    input_table["season"] = dlg:add_text_input(file_info.season or "", 3, 3, 1, 1)
    input_table["episode"] = dlg:add_text_input(file_info.episode or "", 4, 3, 1, 1)
    dlg:add_button("Search by Name", function() do_search("name") end, 5, 3, 1, 1)

    -- Row 4: Interactive Results List (Tabular Formatted Candidates)
    input_table["mainlist"] = dlg:add_list(1, 4, 5, 1)

    -- Row 5: Dark Slate Telemetry & Sync Console
    input_table["message"] = dlg:add_label("", 1, 5, 5, 1)
    set_console_message(
        "<font color=\"#38bdf8\"><b>READY</b></font> &nbsp;|&nbsp; Select a search mode or click <b>1-Click Auto-Sync</b>",
        "Priority order: <b>1. Certified Ground-Truth (pHash)</b> &rarr; <b>2. Local Catalog (.srt)</b> &rarr; <b>3. OpenSubtitles REST</b>.",
        "#38bdf8"
    )

    -- Row 6: Action Toolbar
    dlg:add_button("1-Click Auto-Sync", one_click_auto_sync, 1, 6, 1, 1)
    dlg:add_button("Download + Sync", download_selection, 2, 6, 1, 1)
    dlg:add_button("Certify Coupling", certify_active_sync, 3, 6, 1, 1)
    dlg:add_button("Status / Help", show_help, 4, 6, 1, 1)
    dlg:add_button("Close", close, 5, 6, 1, 1)
end

function show_help()
    check_daemon_status()
    set_console_message(
        "<font color=\"#38bdf8\"><b>WORKFLOW &amp; DIAGNOSTICS</b></font> &nbsp;|&nbsp; <font color=\"#94a3b8\">dbsrt Studio Edition v2.0</font>",
        "<b>1. Smart Search / Search by pHash</b>: Identifies video via 64-bit DCT-II perceptual hashes.<br/>"
        .. "<b>2. Download + Sync</b>: Downloads selected track (or #01 by default) and applies pHash shift.<br/>"
        .. "<b>3. Certify Coupling</b>: Registers verified (Video &lt;-&gt; SRT) ground-truth coupling in PostgreSQL 17 &amp; couplings.json.",
        "#818cf8"
    )
end

function check_daemon_status()
    discover_server()
    local host, port = parse_url(server_url)
    local body, _ = send_http_request(host, port, "GET", "/v1/status")
    if not body then
        daemon_state.online = false
        update_header_banner()
        set_console_message(
            "<font color=\"#fb7185\"><b>DAEMON OFFLINE</b></font>",
            "Cannot reach <b>" .. server_url .. "</b>. Start the stack with <code>task stack:up</code>.",
            "#fb7185"
        )
        return false
    end

    daemon_state.online = true
    daemon_state.version = string.match(body, '"version":%s*"([^"]+)"') or "0.3.0"
    daemon_state.phash_count = string.match(body, '"indexed_phash_count":%s*(%d+)') or "0"
    daemon_state.uptime = string.match(body, '"uptime_seconds":%s*(%d+)') or "0"

    update_header_banner()
    set_console_message(
        string.format("<font color=\"#34d399\"><b>DAEMON READY</b></font> &nbsp;|&nbsp; <font color=\"#94a3b8\">dbsrt-server v%s (%ss uptime)</font>", daemon_state.version, daemon_state.uptime),
        string.format("Indexed memory-mapped fingerprints: <b>%s .phash files</b> &nbsp;|&nbsp; Endpoint: <b>%s</b>", daemon_state.phash_count, server_url),
        "#34d399"
    )
    return true
end

function format_source_tag(source)
    local s = (source or ""):lower()
    if s == "certified" then
        return "[CERTIFIED pHash]"
    elseif s == "catalog" then
        return "[LOCAL CATALOG]  "
    elseif s == "container" then
        return "[MKV TRACK]      "
    elseif s == "opensubtitles" then
        return "[OPENSUBTITLES]  "
    end
    return "[" .. (source or "UNKNOWN"):upper() .. "]"
end

-- Search Execution
function do_search(mode)
    update_file_info()
    update_header_banner()
    if not file_info.has_input or not file_info.path then
        set_console_message(
            "<font color=\"#fb7185\"><b>NO MEDIA PLAYING</b></font>",
            "Start playing a video file in VLC before running subtitle search.",
            "#fb7185"
        )
        return 0
    end

    local lang_idx = input_table["language"]:get_value()
    local lang_code = (languages[lang_idx] and languages[lang_idx][1]) or "fr"
    local host, port = parse_url(server_url)

    local title = input_table["title"]:get_text() or file_info.title
    local season = tonumber(input_table["season"]:get_text())
    local episode = tonumber(input_table["episode"]:get_text())

    local json_parts = {
        string.format('"media_path":"%s"', file_info.path:gsub('\\', '\\\\')),
        string.format('"language":"%s"', lang_code),
        string.format('"search_mode":"%s"', mode or "all")
    }

    if (mode == "name" or mode == "all") and title and title ~= "" then
        table.insert(json_parts, string.format('"query_title":"%s"', title:gsub('"', '\\"')))
    end
    if season then
        table.insert(json_parts, string.format('"season":%d', season))
    end
    if episode then
        table.insert(json_parts, string.format('"episode":%d', episode))
    end

    local payload = "{" .. table.concat(json_parts, ",") .. "}"

    set_console_message(
        string.format("<font color=\"#38bdf8\"><b>SEARCHING (%s)...</b></font>", (mode or "all"):upper()),
        string.format("Querying <b>%s/v1/search</b> for language <b>[%s]</b>...", server_url, lang_code:upper()),
        "#38bdf8"
    )

    local body, err = send_http_request(host, port, "POST", "/v1/search", payload)
    if not body then
        set_console_message(
            "<font color=\"#fb7185\"><b>SEARCH ERROR</b></font>",
            err or "No response from dbsrt-server daemon.",
            "#fb7185"
        )
        notify_osd("dbsrt: Search failed", 3)
        return 0
    end

    local matched_media = string.match(body, '"matched_media":%s*"([^"]+)"')
    local moviehash = string.match(body, '"moviehash":%s*"([^"]+)"')

    candidates_store = {}
    local mainlist = input_table["mainlist"]
    mainlist:clear()

    local count = 0
    for cand_str in string.gmatch(body, '{[^{}]+}') do
        local source = string.match(cand_str, '"source":%s*"([^"]+)"')
        local cand_lang = string.match(cand_str, '"language":%s*"([^"]+)"')
        local cand_title = string.match(cand_str, '"title":%s*"([^"]+)"')
        local cand_id = string.match(cand_str, '"candidate_id":%s*"([^"]+)"')
        local details = string.match(cand_str, '"details":%s*"([^"]+)"')

        if source and cand_title then
            count = count + 1
            local item = {
                index = count,
                source = source,
                language = cand_lang or lang_code,
                title = cand_title,
                candidate_id = cand_id,
                details = details,
            }
            table.insert(candidates_store, item)

            local detail_suffix = ""
            if details and details ~= "" then
                local short_detail = details
                local conf_badge = string.match(details, "%((%d+%% conf)%)")
                local conf_str = conf_badge and (" (" .. conf_badge .. ")") or ""
                if string.find(details, "PostgreSQL", 1, true) then
                    local cid = string.match(details, "#(%d+)") or "1"
                    short_detail = "PostgreSQL #" .. cid .. conf_str
                elseif string.find(details, "Certified Coupling", 1, true) or string.find(details, "couplings.json", 1, true) then
                    short_detail = "Zero-DB Registry" .. conf_str
                elseif #details > 28 then
                    short_detail = string.sub(details, 1, 25) .. "..."
                end
                detail_suffix = "  |  " .. short_detail
            end

            local display_label = string.format(
                "#%02d   %s   [%s]   %s%s",
                count,
                format_source_tag(source),
                (cand_lang or "?"):upper(),
                cand_title,
                detail_suffix
            )
            mainlist:add_value(display_label, count)
        end
    end

    local extra_info = ""
    if matched_media then
        extra_info = string.format(" &nbsp;|&nbsp; <font color=\"#94a3b8\">pHash Ref:</font> <font color=\"#38bdf8\"><b>%s</b></font>", matched_media)
    elseif moviehash then
        extra_info = string.format(" &nbsp;|&nbsp; <font color=\"#94a3b8\">MovieHash:</font> <code>%s</code>", moviehash)
    end

    if count == 0 then
        mainlist:add_value("No subtitle candidates found for current criteria.", 1)
        if mode == "phash" and not matched_media then
            set_console_message(
                "<font color=\"#fbbf24\"><b>PHASH NOT INDEXED: 0 RESULTS</b></font>" .. extra_info,
                "This video has no certified <code>.phash</code> reference in DB yet. Click <b>Smart Search (All)</b> or <b>Search by Name</b>, then <b>Certify Coupling</b>.",
                "#fbbf24"
            )
        else
            set_console_message(
                "<font color=\"#fbbf24\"><b>SEARCH COMPLETE: 0 RESULTS</b></font>" .. extra_info,
                "Try switching Language to <b>All Languages (all)</b> or clicking <b>Smart Search (All)</b>.",
                "#fbbf24"
            )
        end
    else
        set_console_message(
            string.format("<font color=\"#34d399\"><b>SEARCH COMPLETE: %d CANDIDATE(S) FOUND</b></font>%s", count, extra_info),
            "Select a candidate in the list (or leave unselected to use <b>#01</b>) and click <b>Download + Sync</b>.",
            "#34d399"
        )
        notify_osd(string.format("dbsrt: Found %d subtitle(s)", count), 3)
    end

    return count
end

-- 1-Click Auto-Sync: Runs Smart Search (or pHash) and automatically downloads Candidate #1
function one_click_auto_sync()
    local count = do_search("all")
    if count and count > 0 then
        download_candidate_at(1)
    end
end

-- Download and Apply Selected Subtitle (falls back to #1 if none explicitly highlighted)
function download_selection()
    local selection = input_table["mainlist"]:get_selection()
    local sel_idx = nil
    for idx, _ in pairs(selection) do
        sel_idx = idx
        break
    end

    -- Ergonomic fallback: if candidates exist and user didn't click a row, pick #1
    if (not sel_idx or sel_idx == 0) and #candidates_store > 0 then
        sel_idx = 1
    end

    if not sel_idx or not candidates_store[sel_idx] then
        set_console_message(
            "<font color=\"#fbbf24\"><b>NO CANDIDATE AVAILABLE</b></font>",
            "Run a search first or select a subtitle track from the list.",
            "#fbbf24"
        )
        return
    end

    download_candidate_at(sel_idx)
end

function download_candidate_at(sel_idx)
    if not candidates_store[sel_idx] then return end
    local candidate = candidates_store[sel_idx]

    set_console_message(
        string.format("<font color=\"#38bdf8\"><b>SYNCHRONIZING CANDIDATE #%02d...</b></font>", sel_idx),
        string.format("Source: <b>%s</b> &nbsp;|&nbsp; Track: <b>%s</b>", candidate.source:upper(), candidate.title),
        "#38bdf8"
    )

    local host, port = parse_url(server_url)
    local payload = string.format(
        '{"media_path":"%s","source":"%s","candidate_id":"%s","language":"%s","resync":true}',
        file_info.path:gsub('\\', '\\\\'),
        candidate.source,
        (candidate.candidate_id or ""):gsub('\\', '\\\\'),
        candidate.language
    )

    local body, err = send_http_request(host, port, "POST", "/v1/download", payload)
    if not body then
        set_console_message(
            "<font color=\"#fb7185\"><b>DOWNLOAD ERROR</b></font>",
            err or "No response from dbsrt-server daemon.",
            "#fb7185"
        )
        notify_osd("dbsrt: Download failed", 3)
        return
    end

    local success = string.match(body, '"success":%s*(%a+)')
    local out_srt = string.match(body, '"output_srt_path":%s*"([^"]+)"')
    local shift_ms = tonumber(string.match(body, '"shift_ms":%s*([%d%.%-]+)')) or 0.0
    local conf = string.match(body, '"confidence_percent":%s*([%d%.]+)') or "100"
    local source = string.match(body, '"source":%s*"([^"]+)"') or candidate.source

    if success == "true" and out_srt and out_srt ~= "" then
        last_downloaded_srt = out_srt
        last_downloaded_lang = candidate.language or "fr"

        pcall(function() vlc.input.add_subtitle(out_srt, true) end)

        local shift_sec = shift_ms / 1000.0
        local shift_desc = string.format("%+.2fs", shift_sec)
        local conf_num = tonumber(conf) or 100.0
        local out_basename = out_srt:match("([^/\\]+)$") or out_srt

        local osd_msg = ""
        if math.abs(shift_ms) > 1 then
            osd_msg = string.format("dbsrt: [%s] Synced (%s, %.1f%%) via %s", candidate.language:upper(), shift_desc, conf_num, source)
        else
            osd_msg = string.format("dbsrt: [%s] Loaded via %s", candidate.language:upper(), source)
        end
        notify_osd(osd_msg, 4)

        set_console_message(
            string.format(
                "<font color=\"#34d399\"><b>SUBTITLE SYNCHRONIZED &amp; ACTIVE</b></font> &nbsp;|&nbsp; <font color=\"#38bdf8\"><b>[%s] %s</b></font>",
                candidate.language:upper(), source:upper()
            ),
            string.format(
                "Temporal Shift: <font color=\"#f8fafc\"><b>%s (%.1f ms)</b></font> &nbsp;|&nbsp; Confidence: <font color=\"#34d399\"><b>%.1f%%</b></font> &nbsp;|&nbsp; File: <code>%s</code>",
                shift_desc, shift_ms, conf_num, out_basename
            ),
            "#34d399"
        )
    else
        local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "Failed to process subtitle"
        set_console_message(
            "<font color=\"#fb7185\"><b>SYNC ERROR</b></font>",
            err_msg,
            "#fb7185"
        )
        notify_osd("dbsrt error: " .. err_msg, 4)
    end
end

-- Certify Active Subtitle Sync (Ground-Truth Video pHash <-> Subtitle Coupling)
function certify_active_sync()
    update_file_info()
    if not file_info.has_input or not file_info.path then
        set_console_message(
            "<font color=\"#fb7185\"><b>NO MEDIA PLAYING</b></font>",
            "Start a video in VLC before certifying a ground-truth coupling.",
            "#fb7185"
        )
        return
    end

    local srt_path = last_downloaded_srt
    local srt_lang = last_downloaded_lang

    if not srt_path and input_table["mainlist"] then
        local selection = input_table["mainlist"]:get_selection()
        for idx, _ in pairs(selection) do
            local cand = candidates_store[idx]
            if cand and (cand.source == "catalog" or cand.source == "certified") and cand.candidate_id then
                srt_path = cand.candidate_id
                srt_lang = cand.language
                break
            end
        end
        if not srt_path and candidates_store[1] and (candidates_store[1].source == "catalog" or candidates_store[1].source == "certified") then
            srt_path = candidates_store[1].candidate_id
            srt_lang = candidates_store[1].language
        end
    end

    if not srt_path or srt_path == "" then
        set_console_message(
            "<font color=\"#fbbf24\"><b>NO SUBTITLE TRACK SELECTED</b></font>",
            "Download or select a local subtitle track first before certifying ground-truth sync.",
            "#fbbf24"
        )
        return
    end

    if not srt_lang or srt_lang == "" then
        local lang_idx = input_table["language"] and input_table["language"]:get_value() or 1
        srt_lang = (languages[lang_idx] and languages[lang_idx][1]) or "fr"
    end

    set_console_message(
        "<font color=\"#38bdf8\"><b>CERTIFYING GROUND-TRUTH COUPLING...</b></font>",
        "Anchoring subtitle cues to video pHash frames in PostgreSQL 17 &amp; couplings.json...",
        "#38bdf8"
    )

    local host, port = parse_url(server_url)
    local payload = string.format(
        '{"media_path":"%s","subtitle_path":"%s","language":"%s","strict":true}',
        file_info.path:gsub('\\', '\\\\'),
        srt_path:gsub('\\', '\\\\'),
        srt_lang
    )

    local body, err = send_http_request(host, port, "POST", "/v1/certify", payload)
    if not body then
        set_console_message(
            "<font color=\"#fb7185\"><b>CERTIFICATION ERROR</b></font>",
            err or "No response from dbsrt-server daemon.",
            "#fb7185"
        )
        notify_osd("dbsrt: Certify failed", 3)
        return
    end

    local success = string.match(body, '"success":%s*(%a+)')
    if success == "true" then
        local media_name = string.match(body, '"media_name":%s*"([^"]+)"') or "?"
        local cue_count = string.match(body, '"cue_count":%s*(%d+)') or "0"
        local sub_frames_count = string.match(body, '"sub_frames_count":%s*(%d+)') or "0"
        local sub_hash = string.match(body, '"subtitles_hash":%s*"([^"]+)"') or ""
        local short_hash = string.sub(sub_hash, 1, 8)

        notify_osd(
            string.format("dbsrt: Certified [%s] (%s cues / %s anchors)", srt_lang:upper(), cue_count, sub_frames_count),
            4
        )
        set_console_message(
            string.format("<font color=\"#34d399\"><b>GROUND-TRUTH COUPLING CERTIFIED [%s]</b></font>", srt_lang:upper()),
            string.format(
                "Media: <b>%s</b> &nbsp;|&nbsp; Cues: <b>%s</b> (%s frame anchors) &nbsp;|&nbsp; SHA256: <code>%s...</code>",
                media_name, cue_count, sub_frames_count, short_hash
            ),
            "#34d399"
        )
    else
        local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "Certification failed"
        set_console_message(
            "<font color=\"#fb7185\"><b>CERTIFICATION REJECTED</b></font>",
            err_msg,
            "#fb7185"
        )
        notify_osd("dbsrt certify error: " .. err_msg, 4)
    end
end
