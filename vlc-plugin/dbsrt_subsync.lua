--[[
  dbsrt_subsync.lua - VLC Extension for Frame-Based Subtitle Auto-Sync

  Integrates with local dbsrt-server daemon via dynamic Service Discovery (/tmp/dbsrt-server.json)
  to automatically retarget and synchronize subtitles by perceptual frame hashing.
--]]

function descriptor()
    return {
        title = "dbsrt Subtitle Auto-Sync",
        version = "0.2.0",
        author = "Lionel ATTY",
        url = "https://github.com/yoyonel/pydbsrt",
        shortdesc = "Visual Frame-Based Subtitle Auto-Sync",
        description = "<div style=\"background-color: #090c10; color: #c9d1d9; padding: 10px;\">"
            .. "<h2>dbsrt Subtitle Auto-Sync</h2>"
            .. "<p>Automatically matches and synchronizes subtitles using perceptual video frame hashing (DCT-II SIMD).</p>"
            .. "<p>Auto-discovers running <code>dbsrt-server</code> or accepts custom URL.</p>"
            .. "</div>",
        capabilities = { "menu", "input-listener" }
    }
end

local dlg = nil
local default_host = "127.0.0.1"
local default_port = 8765
local server_url = "http://127.0.0.1:8765"
local server_info = "Not discovered"

local lbl_status = nil
local txt_lang = nil
local txt_server_url = nil
local btn_sync = nil

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
    return { "Auto-Sync Subtitle", "Settings" }
end

function trigger_menu(id)
    activate()
end

function activate()
    discover_server()
    create_dialog()
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

function get_current_media_path()
    local item = vlc.input.item()
    if not item then
        return nil
    end
    local uri = item:uri()
    if not uri then
        return nil
    end
    -- Decode file:// URI
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

function create_dialog()
    dlg = vlc.dialog("dbsrt Subtitle Auto-Sync")

    local media_path = get_current_media_path() or "No active video in VLC (start playing a video)"
    dlg:add_label("<b>Active Video:</b>", 1, 1, 4, 1)
    dlg:add_label("<span style=\"color:#58a6ff;\">" .. media_path .. "</span>", 1, 2, 4, 1)

    dlg:add_label("<b>Language (auto / en / fr / all):</b>", 1, 3, 4, 1)
    txt_lang = dlg:add_text_input("auto", 1, 4, 4, 1)

    dlg:add_label("<b>dbsrt-server URL (auto-discovered):</b>", 1, 5, 2, 1)
    txt_server_url = dlg:add_text_input(server_url, 3, 5, 2, 1)

    btn_sync = dlg:add_button("⚡ Auto-Sync Subtitle", do_sync_subtitles, 1, 6, 2, 1)
    dlg:add_button("🔄 Refresh Server", do_refresh_server, 3, 6, 1, 1)
    dlg:add_button("Close", close, 4, 6, 1, 1)

    lbl_status = dlg:add_label("<span style=\"color:#8b949e;\">Status: " .. server_info .. "</span>", 1, 7, 4, 1)
end

function do_refresh_server()
    if discover_server() then
        txt_server_url:set_text(server_url)
        lbl_status:set_text("<span style=\"color:#3fb950;\">✓ " .. server_info .. "</span>")
    else
        lbl_status:set_text("<span style=\"color:#d29922;\">⚠ No /tmp/dbsrt-server.json found. Check 'task server:run'.</span>")
    end
end

function do_sync_subtitles()
    local media_path = get_current_media_path()
    if not media_path then
        lbl_status:set_text("<span style=\"color:#f85149;\">Error: No video currently playing in VLC!</span>")
        return
    end

    local lang = txt_lang:get_text()
    if not lang or lang == "" then
        lang = "auto"
        txt_lang:set_text(lang)
    end

    local current_url = txt_server_url:get_text()
    if not current_url or current_url == "" then
        current_url = server_url
    end

    local host, port = parse_url(current_url)

    lbl_status:set_text("<span style=\"color:#58a6ff;\">Recognizing media and auto-syncing via " .. host .. ":" .. port .. "...</span>")

    -- Build JSON payload for /v1/autosync (Cas 1: zero-subtitle on client)
    local payload = string.format(
        '{"media_path":"%s","language":"%s","search_distance":0}',
        media_path:gsub('\\', '\\\\'),
        lang
    )

    -- Connect via TCP
    local fd = vlc.net.connect_tcp(host, port)
    if fd < 0 then
        lbl_status:set_text(string.format(
            "<span style=\"color:#f85149;\">Error: Cannot connect to %s:%d! Check 'task server:run'.</span>",
            host, port
        ))
        return
    end

    local req = "POST /v1/autosync HTTP/1.1\r\n"
        .. "Host: " .. host .. ":" .. port .. "\r\n"
        .. "Content-Type: application/json\r\n"
        .. "Content-Length: " .. string.len(payload) .. "\r\n"
        .. "Connection: close\r\n\r\n"
        .. payload

    vlc.net.send(fd, req)
    local resp = ""
    for _ = 1, 100 do
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

    -- Extract JSON body
    local _, body_start = string.find(resp, "\r\n\r\n")
    if not body_start then
        lbl_status:set_text("<span style=\"color:#f85149;\">Error: Invalid response from server</span>")
        return
    end
    local json_body = string.sub(resp, body_start + 1)

    -- Simple pattern matching for JSON fields
    local success = string.match(json_body, '"success":%s*(%a+)')
    local shift_ms = string.match(json_body, '"shift_ms":%s*([%d%.%-]+)')
    local srt_out = string.match(json_body, '"output_srt_path":%s*"([^"]+)"')
    local confidence = string.match(json_body, '"confidence_percent":%s*([%d%.]+)')
    local matched = string.match(json_body, '"matched_media":%s*"([^"]+)"') or "unknown"
    local res_lang = string.match(json_body, '"language":%s*"([^"]+)"') or lang

    if success == "true" and srt_out and srt_out ~= "" then
        -- Inject into active VLC video stream
        vlc.input.add_subtitle(srt_out, true)
        lbl_status:set_text(string.format(
            "<span style=\"color:#3fb950;\"><b>✓ Synchronized!</b> Matched: <i>%s</i> [%s]<br/>Shift: %.1f ms | Conf: %s%%</span><br/><small>%s</small>",
            matched,
            res_lang,
            tonumber(shift_ms) or 0,
            confidence or "100",
            srt_out
        ))
    else
        local err_msg = string.match(json_body, '"error":%s*"([^"]+)"') or "Failed to synchronize"
        lbl_status:set_text("<span style=\"color:#f85149;\">Error: " .. err_msg .. "</span>")
    end
end
