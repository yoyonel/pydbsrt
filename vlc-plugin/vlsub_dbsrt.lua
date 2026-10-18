--[[
  vlsub_dbsrt.lua - VLSub Enhanced Fork with Modern dbsrt Backend & Video Sync
  Based on VLSub by Guillaume Le Maout (GPLv2)
  Enhanced by Lionel ATTY for pydbsrt (Rust Axum Daemon, Multi-providers & pHash Sync)
--]]

function descriptor()
    return {
        title = "VLSub (dbsrt Enhanced)",
        version = "1.0.0",
        author = "Lionel ATTY & Guillaume Le Maout",
        url = "https://github.com/yoyonel/pydbsrt",
        shortdesc = "VLSub with Modern REST API & pHash Frame-Based Sync",
        description = "<div style=\"background-color: #0d1117; color: #c9d1d9; padding: 10px;\">"
            .. "<h2>VLSub (dbsrt Enhanced)</h2>"
            .. "<p>Enhanced fork of legacy VLSub: Modern REST API backend, multi-source search "
            .. "(embedded MKV container tracks, local catalog, OpenSubtitles REST), and automatic "
            .. "perceptual hashing (pHash) timeline resynchronization.</p>"
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

local file_info = {
    has_input = false,
    uri = nil,
    path = nil,
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
    { "fr", "French" },
    { "en", "English" },
    { "all", "All" },
    { "es", "Spanish" },
    { "de", "German" },
    { "it", "Italian" },
    { "pt", "Portuguese" },
    { "ru", "Russian" },
    { "ja", "Japanese" },
    { "zh", "Chinese" }
}

-- OSD feedback
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
        default_port = tonumber(port) or 8765
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
        return nil, "Failed to send request"
    end

    local resp = ""
    local pollfds = { [fd] = vlc.net.POLLIN }

    for _ = 1, 50 do
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
        return nil, "Invalid HTTP response from server"
    end
    return string.sub(resp, header_end + 4), nil
end

-- Media file information extraction (VLSub style)
function update_file_info()
    local item = vlc.input.item()
    if not item then
        file_info.has_input = false
        file_info.uri = nil
        file_info.path = nil
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
    fname = fname:gsub("%.%w+$", "")
    file_info.clean_name = fname:gsub("[%._]", " ")

    -- Extract Season & Episode
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
    end
end

-- GUI Interface (VLSub Layout + Search by pHash)
function create_dialog()
    dlg = vlc.dialog("VLSub (dbsrt Enhanced)")

    -- Row 1: Language, Legacy Search by Hash & New Search by pHash
    dlg:add_label("<b>Subtitles language:</b>", 1, 1, 1, 1)
    input_table["language"] = dlg:add_dropdown(2, 1, 1, 1)
    for idx, lang in ipairs(languages) do
        input_table["language"]:add_value(lang[2], idx)
    end

    dlg:add_button("Search by hash", function() do_search("hash") end, 3, 1, 1, 1)
    dlg:add_button("Search by pHash", function() do_search("phash") end, 4, 1, 1, 1)

    -- Row 2: Title & Search by Name
    dlg:add_label("<b>Title:</b>", 1, 2, 1, 1)
    input_table["title"] = dlg:add_text_input(file_info.title or "", 2, 2, 2, 1)
    dlg:add_button("Search by name", function() do_search("name") end, 4, 2, 1, 1)

    -- Row 3: Season & Episode
    dlg:add_label("<b>Season:</b>", 1, 3, 1, 1)
    input_table["season"] = dlg:add_text_input(file_info.season or "", 2, 3, 1, 1)
    dlg:add_label("<b>Episode:</b>", 3, 3, 1, 1)
    input_table["episode"] = dlg:add_text_input(file_info.episode or "", 4, 3, 1, 1)

    -- Row 4: Results List (Interactive selection)
    input_table["mainlist"] = dlg:add_list(1, 4, 4, 1)

    -- Row 5: Message & Status Label
    input_table["message"] = dlg:add_label("Ready. Play a media and search for subtitles.", 1, 5, 4, 1)

    -- Row 6: Action Buttons
    dlg:add_button("Help", show_help, 1, 6, 1, 1)
    dlg:add_button("Status", check_daemon_status, 2, 6, 1, 1)
    dlg:add_button("Download selection", download_selection, 3, 6, 1, 1)
    dlg:add_button("Certify sync", certify_active_sync, 4, 6, 1, 1)
end

function show_help()
    if input_table["message"] then
        input_table["message"]:set_text(
            "<span style=\"color:#8b949e;\"><b>VLSub dbsrt Help:</b><br/>"
            .. "1. Start your video in VLC.<br/>"
            .. "2. Click <b>Search by pHash</b> (visual frame fingerprint), <b>Search by hash</b> (OpenSubtitles MovieHash), or <b>Search by name</b>.<br/>"
            .. "3. Select a subtitle in the list and click <b>Download selection</b>.<br/>"
            .. "4. Click <b>Certify sync</b> once verified to register the ground-truth (video pHash &lt;-&gt; subtitle) coupling!</span>"
        )
    end
end

function check_daemon_status()
    discover_server()
    local host, port = parse_url(server_url)
    local body, err = send_http_request(host, port, "GET", "/v1/status")
    if not body then
        local msg = "<span style=\"color:#f85149;\">⚠ dbsrt-server offline (run 'task server:daemon')</span>"
        if input_table["message"] then input_table["message"]:set_text(msg) end
        return false
    end

    local ver = string.match(body, '"version":%s*"([^"]+)"') or "?"
    local count = string.match(body, '"indexed_phash_count":%s*(%d+)') or "0"
    local uptime = string.match(body, '"uptime_seconds":%s*(%d+)') or "0"

    local msg = string.format(
        "<span style=\"color:#3fb950;\">✓ dbsrt-server v%s online (%ss uptime, %s indexed medias)</span>",
        ver, uptime, count
    )
    if input_table["message"] then input_table["message"]:set_text(msg) end
    return true
end

-- Search Execution
function do_search(mode)
    update_file_info()
    if not file_info.has_input or not file_info.path then
        if input_table["message"] then
            input_table["message"]:set_text("<span style=\"color:#f85149;\">Error: No video playing in VLC!</span>")
        end
        return
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

    if mode == "name" and title and title ~= "" then
        table.insert(json_parts, string.format('"query_title":"%s"', title:gsub('"', '\\"')))
    end
    if season then
        table.insert(json_parts, string.format('"season":%d', season))
    end
    if episode then
        table.insert(json_parts, string.format('"episode":%d', episode))
    end

    local payload = "{" .. table.concat(json_parts, ",") .. "}"

    if input_table["message"] then
        input_table["message"]:set_text("<span style=\"color:#58a6ff;\">Searching subtitles (" .. mode .. ")...</span>")
    end

    local body, err = send_http_request(host, port, "POST", "/v1/search", payload)
    if not body then
        local msg = "<span style=\"color:#f85149;\">Search error: " .. (err or "No response from daemon") .. "</span>"
        if input_table["message"] then input_table["message"]:set_text(msg) end
        notify_osd("✗ dbsrt search failed", 3)
        return
    end

    local matched_media = string.match(body, '"matched_media":%s*"([^"]+)"')
    local moviehash = string.match(body, '"moviehash":%s*"([^"]+)"')

    candidates_store = {}
    local mainlist = input_table["mainlist"]
    mainlist:clear()

    -- Parse JSON candidates
    local count = 0
    -- Pattern matching individual candidate objects
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

            local display_label = string.format(
                "[%s] [%s] %s",
                source:upper(),
                (cand_lang or "?"):upper(),
                cand_title
            )
            mainlist:add_value(display_label, count)
        end
    end

    local extra_info = ""
    if mode == "phash" and matched_media then
        extra_info = string.format(" <i>(pHash match: %s)</i>", matched_media)
    elseif mode == "hash" and moviehash then
        extra_info = string.format(" <i>(MovieHash: %s)</i>", moviehash)
    end

    if count == 0 then
        mainlist:add_value("No subtitles found", 1)
        if input_table["message"] then
            input_table["message"]:set_text("<b>Search complete:</b> 0 results." .. extra_info)
        end
    else
        if input_table["message"] then
            input_table["message"]:set_text(string.format("<b>Search complete:</b> Found %d subtitle(s).%s", count, extra_info))
        end
        notify_osd(string.format("dbsrt: Found %d subtitle(s)", count), 3)
    end
end

-- Download and Apply Selected Subtitle
function download_selection()
    local selection = input_table["mainlist"]:get_selection()
    local sel_idx = nil
    for idx, _ in pairs(selection) do
        sel_idx = idx
        break
    end

    if not sel_idx or sel_idx == 0 or not candidates_store[sel_idx] then
        if input_table["message"] then
            input_table["message"]:set_text("<span style=\"color:#f85149;\">Please select a subtitle from the list first.</span>")
        end
        return
    end

    download_candidate_at(sel_idx)
end

function download_candidate_at(sel_idx)
    if not candidates_store[sel_idx] then return end
    local candidate = candidates_store[sel_idx]
    if input_table["message"] then
        input_table["message"]:set_text("<span style=\"color:#58a6ff;\">Downloading & syncing selection...</span>")
    end

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
        local msg = "<span style=\"color:#f85149;\">Download error: " .. (err or "No response from daemon") .. "</span>"
        if input_table["message"] then input_table["message"]:set_text(msg) end
        notify_osd("✗ Download failed", 3)
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

        -- Attach to VLC video stream
        local ok, add_err = pcall(function() vlc.input.add_subtitle(out_srt, true) end)

        local shift_sec = shift_ms / 1000.0
        local shift_desc = string.format("%+.2fs", shift_sec)
        local conf_num = tonumber(conf) or 100.0
        local out_basename = out_srt:match("([^/\\]+)$") or out_srt

        local osd_msg = ""
        if math.abs(shift_ms) > 1 then
            osd_msg = string.format("✓ dbsrt: [%s] Synced (%s, %.1f%%) via %s", candidate.language:upper(), shift_desc, conf_num, source)
        else
            osd_msg = string.format("✓ dbsrt: [%s] Loaded via %s", candidate.language:upper(), source)
        end
        notify_osd(osd_msg, 4)

        if input_table["message"] then
            input_table["message"]:set_text(string.format(
                "<span style=\"color:#3fb950;\"><b>✓ Subtitle Loaded!</b> [%s]<br/>"
                .. "Source: <b>%s</b> | Resync Shift: <b>%s</b> (%.1f ms, %.1f%% conf)</span><br/>"
                .. "<small style=\"color:#8b949e;\">%s</small>",
                candidate.language:upper(), source, shift_desc, shift_ms, conf_num, out_basename
            ))
        end
    else
        local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "Failed to process subtitle"
        if input_table["message"] then
            input_table["message"]:set_text("<span style=\"color:#f85149;\">Error: " .. err_msg .. "</span>")
        end
        notify_osd("✗ dbsrt error: " .. err_msg, 4)
    end
end

-- Certify Active Subtitle Sync (Ground-Truth Video pHash <-> Subtitle Coupling)
function certify_active_sync()
    update_file_info()
    if not file_info.has_input or not file_info.path then
        if input_table["message"] then
            input_table["message"]:set_text("<span style=\"color:#f85149;\">Error: No video playing in VLC!</span>")
        end
        return
    end

    local srt_path = last_downloaded_srt
    local srt_lang = last_downloaded_lang

    -- Fallback: if no subtitle was downloaded yet in this session, check selected catalog/certified item
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
    end

    if not srt_path or srt_path == "" then
        if input_table["message"] then
            input_table["message"]:set_text(
                "<span style=\"color:#f85149;\">Download or select a local subtitle first before certifying sync.</span>"
            )
        end
        return
    end

    if not srt_lang or srt_lang == "" then
        local lang_idx = input_table["language"] and input_table["language"]:get_value() or 1
        srt_lang = (languages[lang_idx] and languages[lang_idx][1]) or "fr"
    end

    if input_table["message"] then
        input_table["message"]:set_text("<span style=\"color:#58a6ff;\">Certifying ground-truth pHash coupling...</span>")
    end

    local host, port = parse_url(server_url)
    local payload = string.format(
        '{"media_path":"%s","subtitle_path":"%s","language":"%s","strict":true}',
        file_info.path:gsub('\\', '\\\\'),
        srt_path:gsub('\\', '\\\\'),
        srt_lang
    )

    local body, err = send_http_request(host, port, "POST", "/v1/certify", payload)
    if not body then
        local msg = "<span style=\"color:#f85149;\">Certify error: " .. (err or "No response from daemon") .. "</span>"
        if input_table["message"] then input_table["message"]:set_text(msg) end
        notify_osd("✗ Certify sync failed", 3)
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
            string.format("✓ Certified [%s] (%s cues / %s frame anchors)", srt_lang:upper(), cue_count, sub_frames_count),
            4
        )
        if input_table["message"] then
            input_table["message"]:set_text(string.format(
                "<span style=\"color:#3fb950;\"><b>✓ Ground-Truth Coupling Certified!</b> [%s]<br/>"
                .. "Media: <b>%s</b> | Cues: <b>%s</b> (%s frame anchors, SHA256: %s…)</span>",
                srt_lang:upper(), media_name, cue_count, sub_frames_count, short_hash
            ))
        end
    else
        local err_msg = string.match(body, '"error":%s*"([^"]+)"') or "Certification failed"
        if input_table["message"] then
            input_table["message"]:set_text("<span style=\"color:#f85149;\">Certify Error: " .. err_msg .. "</span>")
        end
        notify_osd("✗ Certify error: " .. err_msg, 4)
    end
end
