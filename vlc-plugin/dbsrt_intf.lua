--[[
  dbsrt_intf.lua - Headless/CLI VLC Lua Interface for Frame-Based Subtitle Auto-Sync

  Can be loaded via cvlc or vlc in headless/CLI mode:
    cvlc --extraintf luaintf --lua-intf dbsrt_intf <video>
--]]

vlc.msg.info("[dbsrt-intf] Module initialized")

local function json_escape(str)
    if not str then return "" end
    local s = tostring(str)
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
local function parse_json_string_field(json_str, key)
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

local function run_sync()
    local item = vlc.input.item()
    if not item then
        vlc.msg.err("[dbsrt-intf] No active item")
        return
    end
    local uri = item:uri()
    local path = vlc.strings.decode_uri(uri):gsub("^file://", "")
    vlc.msg.info("[dbsrt-intf] Active video: " .. path)

    -- Cas 1: Zero-subtitle client. Let server identify media & fetch catalog subtitle.
    local lang = os.getenv("DBSRT_LANG") or "auto"
    vlc.msg.info("[dbsrt-intf] Requesting auto-sync for language: " .. lang)

    -- Read dynamic discovery file
    local disc = io.open("/tmp/dbsrt-server.json", "r")
    local port = 8765
    local host = "127.0.0.1"
    if disc then
        local content = disc:read("*all")
        disc:close()
        port = tonumber(string.match(content, '"port":%s*(%d+)')) or 8765
        host = string.match(content, '"host":%s*"([^"]+)"') or "127.0.0.1"
    end

    vlc.msg.info(string.format("[dbsrt-intf] Connecting to server %s:%d", host, port))
    local fd = vlc.net.connect_tcp(host, port)
    if fd < 0 then
        vlc.msg.err("[dbsrt-intf] TCP connect failed")
        return
    end

    local payload = string.format(
        '{"media_path":"%s","language":"%s","search_distance":0}',
        json_escape(path),
        json_escape(lang)
    )

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

    local header_end = string.find(resp, "\r\n\r\n")
    if header_end then
        local body = string.sub(resp, header_end + 4)
        local out_srt = parse_json_string_field(body, "output_srt_path") or string.match(body, '"output_srt_path":%s*"([^"]+)"')
        local shift_ms = string.match(body, '"shift_ms":%s*([%d%.%-]+)')
        local conf = string.match(body, '"confidence_percent":%s*([%d%.]+)')
        local matched = string.match(body, '"matched_media":%s*"([^"]+)"') or "unknown"
        local source = string.match(body, '"source":%s*"([^"]+)"') or "catalog"
        if out_srt and out_srt ~= "" then
            local ok, err = pcall(function() vlc.input.add_subtitle(out_srt, true) end)
            if ok then
                vlc.msg.info(string.format("[dbsrt-intf] SUCCESS! Media matched: '%s' -> Subtitle attached: %s (source: %s, shift: %s ms, conf: %s%%)", matched, out_srt, source, shift_ms or "0", conf or "100"))
            else
                vlc.msg.warn(string.format("[dbsrt-intf] add_subtitle notification (%s) -> Subtitle ready at: %s (source: %s, shift: %s ms)", tostring(err), out_srt, source, shift_ms or "0"))
            end
            if vlc.osd and vlc.osd.message then
                pcall(function() vlc.osd.message(string.format("✓ dbsrt: [%s] Synced (%s ms) via %s", lang:upper(), shift_ms or "0", source), 1, "top-right", 3000000) end)
            end
            if os.getenv("DBSRT_AUTOKILL") == "1" then
                vlc.msg.info("[dbsrt-intf] Event-driven autokill triggered: subtitle processed -> quitting VLC")
                vlc.misc.quit()
            end
        else
            local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "Failed to match/sync"
            vlc.msg.err("[dbsrt-intf] Sync error from daemon: " .. err_msg)
            if os.getenv("DBSRT_AUTOKILL") == "1" then
                vlc.msg.info("[dbsrt-intf] Event-driven autokill triggered (match completed without subtitle) -> quitting VLC")
                vlc.misc.quit()
            end
        end
    else
        vlc.msg.err("[dbsrt-intf] No valid HTTP response received from server")
    end
end

-- Wait for playback
for _ = 1, 50 do
    if vlc.playlist.status() == "playing" and vlc.input.item() then
        run_sync()
        break
    end
    vlc.misc.mwait(vlc.misc.mdate() + 100000) -- 100ms
end
