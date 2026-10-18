--[[
  dbsrt_subsync.lua - VLC Extension for Frame-Based Subtitle Auto-Sync (v2.0)

  Features:
  - Background zero-click synchronization on media play (input_changed listener)
  - Native On-Screen Display (OSD) feedback directly over active video
  - Multi-candidate search (/v1/search) across container tracks, local catalog & OpenSubtitles
  - Interactive GUI panel with language quick-pick and server status telemetry (/v1/status)
--]]

function descriptor()
    return {
        title = "dbsrt Subtitle Auto-Sync",
        version = "0.3.0",
        author = "Lionel ATTY",
        url = "https://github.com/yoyonel/pydbsrt",
        shortdesc = "Visual Frame-Based Subtitle Auto-Sync & OSD",
        description = "<div style=\"background-color: #090c10; color: #c9d1d9; padding: 10px;\">"
            .. "<h2>dbsrt Subtitle Auto-Sync (v2.0)</h2>"
            .. "<p>Visual frame-based subtitle synchronization engine (DCT-II SIMD & BK-Tree).</p>"
            .. "<p>Zero-click auto-sync, OSD feedback, and multi-source fallback.</p>"
            .. "</div>",
        capabilities = { "menu", "input-listener" }
    }
end

local dlg = nil
local default_host = "127.0.0.1"
local default_port = 8765
local server_url = "http://127.0.0.1:8765"
local server_info = "Checking..."
local auto_sync_enabled = true
local last_synced_media = nil
local osd_channel = nil

local lbl_status = nil
local lbl_media = nil
local txt_lang = nil
local txt_server_url = nil
local btn_toggle_auto = nil

-- OSD feedback helper
function notify_osd(msg, duration_sec, pos)
    duration_sec = duration_sec or 3
    pos = pos or "top-right"
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
        vlc.msg.info("[dbsrt-subsync] " .. msg)
    end
end

-- Reads dynamic discovery file written by dbsrt-server
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
    local pid = string.match(content, '"pid":%s*(%d+)')

    if url and port then
        server_url = url
        default_host = host or "127.0.0.1"
        default_port = tonumber(port) or 8765
        server_info = string.format("Found daemon (PID %s) at %s", pid or "?", url)
        return true
    end
    return nil
end

function menu()
    return { "Auto-Sync Subtitle", "Search Candidates", "Settings" }
end

function trigger_menu(id)
    if id == 1 then
        activate()
        do_sync_subtitles(false)
    elseif id == 2 then
        activate()
        do_search_candidates()
    else
        activate()
    end
end

function activate()
    discover_server()
    create_dialog()
    ping_server()
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

-- Input listener: triggered when a new file starts playing in VLC
function input_changed()
    if not auto_sync_enabled then
        return
    end

    local path = get_current_media_path()
    if not path or path == "" or path == last_synced_media then
        return
    end

    last_synced_media = path
    notify_osd("⚡ dbsrt: Auto-syncing subtitles...", 2)

    -- Run auto-sync non-blocking in background
    do_sync_subtitles(true)
end

function get_current_media_path()
    local item = vlc.input.item()
    if not item then
        return nil
    end
    local uri = item:uri()
    if not uri then
        return nil
    end
    local path = vlc.strings.decode_uri(uri)
    path = path:gsub("^file://", "")
    return path
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

-- HTTP TCP request helper
function send_http_request(host, port, method, path, payload)
    local fd = vlc.net.connect_tcp(host, port)
    if fd < 0 then
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

    vlc.net.send(fd, req)
    local resp = ""
    for _ = 1, 120 do
        local p = vlc.net.poll({ [fd] = vlc.net.POLLIN })
        if p > 0 then
            local chunk = vlc.net.recv(fd, 4096)
            if chunk and string.len(chunk) > 0 then
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
            end
        end
        vlc.misc.mwait(vlc.misc.mdate() + 10000) -- 10ms
    end
    vlc.net.close(fd)

    local header_end = string.find(resp, "\r\n\r\n")
    if not header_end then
        return nil, "Invalid HTTP response from server"
    end
    return string.sub(resp, header_end + 4), nil
end

function ping_server()
    discover_server()
    local host, port = parse_url(server_url)
    local body, err = send_http_request(host, port, "GET", "/v1/status")
    if not body then
        server_info = "<span style=\"color:#f85149;\">⚠ Server offline (" .. (err or "no response") .. ")</span>"
        if lbl_status then lbl_status:set_text(server_info) end
        return false
    end

    local status = string.match(body, '"status":%s*"([^"]+)"')
    local ver = string.match(body, '"version":%s*"([^"]+)"')
    local count = string.match(body, '"indexed_phash_count":%s*(%d+)') or "0"
    local uptime = string.match(body, '"uptime_seconds":%s*(%d+)') or "0"

    if status == "ok" then
        server_info = string.format(
            "<span style=\"color:#3fb950;\">✓ dbsrt-server v%s (uptime: %ss, %s medias in index)</span>",
            ver or "?", uptime, count
        )
        if lbl_status then lbl_status:set_text(server_info) end
        return true
    end
    return false
end

function create_dialog()
    dlg = vlc.dialog("dbsrt Subtitle Auto-Sync v2.0")

    local media_path = get_current_media_path() or "No active video (play a video in VLC)"
    dlg:add_label("<b>Active Media:</b>", 1, 1, 1, 1)
    lbl_media = dlg:add_label("<span style=\"color:#58a6ff;\">" .. media_path .. "</span>", 2, 1, 4, 1)

    dlg:add_label("<b>Language:</b>", 1, 2, 1, 1)
    txt_lang = dlg:add_text_input("fr", 2, 2, 1, 1)
    dlg:add_button("FR", function() txt_lang:set_text("fr") end, 3, 2, 1, 1)
    dlg:add_button("EN", function() txt_lang:set_text("en") end, 4, 2, 1, 1)
    dlg:add_button("AUTO", function() txt_lang:set_text("auto") end, 5, 2, 1, 1)

    dlg:add_label("<b>Auto-Sync:</b>", 1, 3, 1, 1)
    local auto_text = auto_sync_enabled and "ON (Auto-sync when media starts)" or "OFF (Manual click only)"
    btn_toggle_auto = dlg:add_button(auto_text, toggle_auto_sync, 2, 3, 4, 1)

    dlg:add_label("<b>Daemon:</b>", 1, 4, 1, 1)
    txt_server_url = dlg:add_text_input(server_url, 2, 4, 3, 1)
    dlg:add_button("🔄 Ping", ping_server, 5, 4, 1, 1)

    dlg:add_button("⚡ Auto-Sync Subtitle", function() do_sync_subtitles(false) end, 1, 5, 2, 1)
    dlg:add_button("🔍 Search Candidates", do_search_candidates, 3, 5, 2, 1)
    dlg:add_button("Close", close, 5, 5, 1, 1)

    lbl_status = dlg:add_label("Status: " .. server_info, 1, 6, 5, 1)
end

function toggle_auto_sync()
    auto_sync_enabled = not auto_sync_enabled
    local auto_text = auto_sync_enabled and "ON (Auto-sync when media starts)" or "OFF (Manual click only)"
    if btn_toggle_auto then
        btn_toggle_auto:set_text(auto_text)
    end
    notify_osd("dbsrt auto-sync: " .. (auto_sync_enabled and "ENABLED" or "DISABLED"), 2)
end

function do_search_candidates()
    local media_path = get_current_media_path()
    if not media_path then
        if lbl_status then
            lbl_status:set_text("<span style=\"color:#f85149;\">Error: No video currently playing!</span>")
        end
        return
    end

    local lang = (txt_lang and txt_lang:get_text()) or "auto"
    if lang == "" then lang = "auto" end
    local current_url = (txt_server_url and txt_server_url:get_text()) or server_url
    local host, port = parse_url(current_url)

    if lbl_status then
        lbl_status:set_text("<span style=\"color:#58a6ff;\">Searching subtitle candidates across sources...</span>")
    end

    local payload = string.format('{"media_path":"%s","language":"%s"}', media_path:gsub('\\', '\\\\'), lang)
    local body, err = send_http_request(host, port, "POST", "/v1/search", payload)
    if not body then
        if lbl_status then
            lbl_status:set_text("<span style=\"color:#f85149;\">Search error: " .. (err or "failed") .. "</span>")
        end
        return
    end

    local total = string.match(body, '"total_candidates":%s*(%d+)') or "0"
    local summary = string.format("<b>Found %s candidate(s):</b><br/>", total)

    -- Parse each candidate JSON block independently of field order
    for cand_str in string.gmatch(body, '{[^{}]+}') do
        local source = string.match(cand_str, '"source":%s*"([^"]+)"')
        local c_lang = string.match(cand_str, '"language":%s*"([^"]+)"')
        local title = string.match(cand_str, '"title":%s*"([^"]+)"')
        if source and title then
            summary = summary .. string.format("• <b>[%s]</b> %s (%s)<br/>", source:upper(), title, c_lang or lang)
        end
    end

    if tonumber(total) == 0 then
        summary = summary .. "<i>No matching subtitles found in container, catalog, or OpenSubtitles.</i>"
    end

    if lbl_status then
        lbl_status:set_text("<span style=\"color:#c9d1d9;\">" .. summary .. "</span>")
    end
    notify_osd(string.format("dbsrt: Found %s subtitle candidate(s)", total), 3)
end

function do_sync_subtitles(is_auto)
    local media_path = get_current_media_path()
    if not media_path then
        if not is_auto and lbl_status then
            lbl_status:set_text("<span style=\"color:#f85149;\">Error: No video currently playing in VLC!</span>")
        end
        return
    end

    local lang = (txt_lang and txt_lang:get_text()) or "auto"
    if not lang or lang == "" then lang = "auto" end

    local current_url = (txt_server_url and txt_server_url:get_text()) or server_url
    local host, port = parse_url(current_url)

    if not is_auto and lbl_status then
        lbl_status:set_text("<span style=\"color:#58a6ff;\">Matching media and retargeting subtitles...</span>")
    end

    local payload = string.format(
        '{"media_path":"%s","language":"%s","search_distance":0}',
        media_path:gsub('\\', '\\\\'),
        lang
    )

    local body, err = send_http_request(host, port, "POST", "/v1/autosync", payload)
    if not body then
        local msg = "Sync failed: " .. (err or "No response from dbsrt-server")
        if not is_auto and lbl_status then
            lbl_status:set_text("<span style=\"color:#f85149;\">" .. msg .. "</span>")
        end
        notify_osd("✗ " .. msg, 3)
        return
    end

    local success = string.match(body, '"success":%s*(%a+)')
    local shift_ms = string.match(body, '"shift_ms":%s*([%d%.%-]+)')
    local srt_out = string.match(body, '"output_srt_path":%s*"([^"]+)"')
    local confidence = string.match(body, '"confidence_percent":%s*([%d%.]+)')
    local matched = string.match(body, '"matched_media":%s*"([^"]+)"') or "unknown"
    local res_lang = string.match(body, '"language":%s*"([^"]+)"') or lang
    local source = string.match(body, '"source":%s*"([^"]+)"') or "catalog"

    if success == "true" and srt_out and srt_out ~= "" then
        -- Inject into active VLC video stream
        local ok, inject_err = pcall(function() vlc.input.add_subtitle(srt_out, true) end)
        local shift_num = tonumber(shift_ms) or 0
        local shift_str = string.format("%+.2fs", shift_num / 1000.0)

        -- OSD Notification directly over video playback
        notify_osd(string.format("✓ dbsrt: [%s] Synced (%s, %s%%) via %s", res_lang:upper(), shift_str, confidence or "100", source), 4)

        if lbl_status then
            lbl_status:set_text(string.format(
                "<span style=\"color:#3fb950;\"><b>✓ Synchronized!</b> [%s] <i>%s</i><br/>"
                .. "Source: <b>%s</b> | Shift: <b>%s</b> (%.1f ms) | Conf: <b>%s%%</b></span><br/>"
                .. "<small style=\"color:#8b949e;\">%s</small>",
                res_lang:upper(), matched, source, shift_str, shift_num, confidence or "100", srt_out
            ))
        end
    else
        local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "No subtitle could be matched"
        notify_osd("✗ dbsrt: " .. err_msg, 4)
        if lbl_status then
            lbl_status:set_text("<span style=\"color:#f85149;\">Error: " .. err_msg .. "</span>")
        end
    end
end
