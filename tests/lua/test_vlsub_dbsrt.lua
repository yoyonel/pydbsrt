--[[
  tests/lua/test_vlsub_dbsrt.lua
  Hermetic Unit Test Suite for VLSub (dbsrt Enhanced) Lua Extension.
  Mocks VLC runtime API to validate parsing, HTTP sockets, and UI state without VLC.
--]]

local tests_run = 0
local tests_passed = 0

local function assert_eq(actual, expected, msg)
    tests_run = tests_run + 1
    if actual == expected then
        tests_passed = tests_passed + 1
    else
        error(string.format("FAILED: %s | Expected: %s, got: %s", msg or "", tostring(expected), tostring(actual)), 2)
    end
end

local function assert_true(val, msg)
    assert_eq(val == true, true, msg)
end

print("=== [VLC Lua Plugin Unit Tests] Mocking LibVLC Environment ===")

-- 1. Hermetic Mock of VLC Global Environment
local mock_dialog_elements = {}
local mock_active_item_uri = "file:///videos/Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4"
local mock_attached_subtitles = {}
local mock_tcp_responses = {}
local mock_last_sent_payload = nil

local function create_mock_widget(w_type, initial_text)
    local w = {
        _type = w_type,
        _text = initial_text or "",
        _values = {},
        _selection = {},
        get_text = function(self) return self._text end,
        set_text = function(self, txt) self._text = txt end,
        add_value = function(self, val, idx)
            idx = idx or (#self._values + 1)
            self._values[idx] = val
        end,
        get_value = function(self) return 1 end,
        clear = function(self)
            self._values = {}
            self._selection = {}
        end,
        get_selection = function(self) return self._selection end
    }
    return w
end

_G.vlc = {
    msg = {
        dbg = function(_) end,
        info = function(_) end,
        warn = function(_) end,
        err = function(_) end
    },
    strings = {
        decode_uri = function(s)
            s = s:gsub("%%20", " ")
            return s
        end
    },
    input = {
        item = function()
            if not mock_active_item_uri then return nil end
            return {
                uri = function() return mock_active_item_uri end
            }
        end,
        add_subtitle = function(path, flag)
            table.insert(mock_attached_subtitles, { path = path, flag = flag })
            return true
        end
    },
    osd = {
        channel_register = function() return 1 end,
        message = function(_, _, _, _) end
    },
    dialog = function(title)
        return {
            _title = title,
            add_label = function(_, text, _, _, _, _)
                local w = create_mock_widget("label", text)
                table.insert(mock_dialog_elements, w)
                return w
            end,
            add_text_input = function(_, text, _, _, _, _)
                local w = create_mock_widget("text_input", text)
                table.insert(mock_dialog_elements, w)
                return w
            end,
            add_dropdown = function(_, _, _, _, _)
                local w = create_mock_widget("dropdown", "")
                table.insert(mock_dialog_elements, w)
                return w
            end,
            add_list = function(_, _, _, _, _)
                local w = create_mock_widget("list", "")
                table.insert(mock_dialog_elements, w)
                return w
            end,
            add_button = function(_, _, _, _, _, _, _)
                local w = create_mock_widget("button", "")
                return w
            end,
            add_html = function(_, html, _, _, _, _)
                local w = create_mock_widget("html", html)
                return w
            end,
            update = function(_) end,
            hide = function(_) end
        }
    end,
    deactivate = function() end,
    -- Strictly nil to verify extension NEVER attempts to call forbidden vlc.misc!
    misc = nil,
    net = {
        POLLIN = 1,
        connect_tcp = function(host, port)
            return 42
        end,
        send = function(fd, req)
            mock_last_sent_payload = req
            return string.len(req)
        end,
        poll = function(pollfds)
            return 1
        end,
        recv = function(fd, max_len)
            if #mock_tcp_responses > 0 then
                local chunk = table.remove(mock_tcp_responses, 1)
                return chunk
            end
            return ""
        end,
        close = function(fd) end
    }
}

-- 2. Load the target extension under test
_G._VLSub_test_hook = true
dofile("vlc-plugin/vlsub_dbsrt.lua")
local S = _G._VLSub_state

print("✓ vlc-plugin/vlsub_dbsrt.lua loaded successfully")

-- 3. Unit Test Cases
local function test_descriptor()
    local d = descriptor()
    assert_eq(d.title, "VLSub (dbsrt Enhanced)", "Descriptor title")
    assert_eq(d.version, "2.1.0", "Descriptor version")
    assert_true(type(d.capabilities) == "table", "Descriptor capabilities")
    print("  [1/7] test_descriptor: PASSED")
end

local function test_parse_url()
    local h1, p1 = S.parse_url("http://127.0.0.1:8765")
    assert_eq(h1, "127.0.0.1", "Host parsed")
    assert_eq(p1, 8765, "Port parsed")

    local h2, p2 = S.parse_url("http://localhost")
    assert_eq(h2, "localhost", "Default port host")
    assert_eq(p2, 80, "Default port fallback to 80")
    print("  [2/7] test_parse_url: PASSED")
end

local function test_update_file_info()
    mock_active_item_uri = "file:///media/The.Boys.S02E03.1080p.AMZN.WEBRip.DDP5.1.x264-NTb.mp4"
    local ok = S.update_file_info()
    assert_true(ok, "Update file info success")
    assert_true(S.file_info.title:find("The Boys") ~= nil, "Parsed title contains 'The Boys'")
    assert_eq(S.file_info.season, "2", "Parsed season S02 -> 2")
    assert_eq(S.file_info.episode, "3", "Parsed episode E03 -> 3")
    print("  [3/7] test_update_file_info: PASSED")
end

local function test_multi_chunk_http_socket_without_misc()
    local body_content = string.rep("x", 5000)
    local json_body = string.format('{"status":"ok","data":"%s"}', body_content)
    local full_http = string.format("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s", string.len(json_body), json_body)

    local chunk1 = full_http:sub(1, 3000)
    local chunk2 = full_http:sub(3001)

    mock_tcp_responses = { chunk1, chunk2 }

    local body, err = S.send_http_request("127.0.0.1", 8765, "GET", "/v1/test", nil)
    assert_true(body ~= nil, "HTTP response body received")
    assert_eq(err, nil, "No HTTP error")
    assert_eq(string.len(body), string.len(json_body), "Reassembled multi-chunk body exact length")
    print("  [4/7] test_multi_chunk_http_socket_without_misc: PASSED")
end

local function test_search_by_phash_populates_mainlist()
    activate()

    local sample_json = [[{
        "matched_media": "Andor",
        "total_candidates": 2,
        "candidates": [
            {
                "source": "certified",
                "language": "fr",
                "title": "Andor.S01E01.1080p.WEB.h264-KOGi.fr.srt",
                "candidate_id": "data/SRT/Andor.S01E01.1080p.WEB.h264-KOGi.fr.srt",
                "shift_ms": 0.0,
                "score": 1.0
            },
            {
                "source": "certified_synced",
                "language": "fr",
                "title": "Andor.S01E01.1080p.WEB.h264-KOGi.fr.srt [Synced -1167.8s]",
                "candidate_id": "data/SRT/Andor.S01E01.1080p.WEB.h264-KOGi.fr.srt",
                "shift_ms": -1167833.33,
                "score": 1.0
            },
            {
                "source": "catalog",
                "language": "fr",
                "title": "Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.fr.srt",
                "candidate_id": "data/SRT/Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.fr.srt",
                "score": 0.95
            }
        ]
    }]]

    local full_http = string.format("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s", string.len(sample_json), sample_json)
    mock_tcp_responses = { full_http }

    S.search_by_phash()

    assert_eq(#S.candidates_store, 3, "Parsed candidates count")
    assert_eq(S.candidates_store[1].source, "certified", "First candidate source")
    assert_eq(S.candidates_store[1].title, "Andor.S01E01.1080p.WEB.h264-KOGi.fr.srt", "First candidate title")
    assert_eq(S.candidates_store[2].source, "certified_synced", "Second candidate source")
    assert_true(S.candidates_store[2].title:find("Synced") ~= nil, "Second candidate title contains Synced")

    local mainlist = S.input_table["mainlist"]
    assert_true(mainlist ~= nil, "Mainlist widget exists")
    assert_eq(#mainlist._values, 3, "Mainlist populated with 3 items")
    assert_true(mainlist._values[1]:find("pHash certified") ~= nil, "Mainlist displays 'pHash certified' tag")
    assert_true(mainlist._values[2]:find("Synced") ~= nil, "Mainlist displays 'Synced' entry")
    print("  [5/7] test_search_by_phash_populates_mainlist: PASSED")
end

local function test_download_selection()
    local mainlist = S.input_table["mainlist"]
    mainlist._selection = { [1] = mainlist._values[1] }

    local download_resp = [[{
        "success": true,
        "output_srt_path": "/media/Andor.S01E01.fr.srt",
        "shift_ms": 150.0,
        "confidence_percent": 98.5
    }]]

    local full_http = string.format("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s", string.len(download_resp), download_resp)
    mock_tcp_responses = { full_http }

    mock_attached_subtitles = {}
    S.download_selection()

    assert_eq(#mock_attached_subtitles, 1, "One subtitle attached to VLC")
    assert_eq(mock_attached_subtitles[1].path, "/media/Andor.S01E01.fr.srt", "Attached subtitle path exact match")
    assert_true(S.input_table["message"]:get_text():find("Subtitles loaded") ~= nil, "Message label confirms loaded")
    print("  [6/7] test_download_selection: PASSED")
end

local function test_help_dialog()
    show_help()
    assert_true(S.input_table["help"] ~= nil, "Help widget displayed")
    local html = S.input_table["help"]:get_text()
    assert_true(html:find("Method 3: Search by pHash") ~= nil, "Help documents Method 3 pHash")
    assert_true(html:find("Method 1: Search by hash") ~= nil, "Help documents Method 1 hash")
    print("  [7/7] test_help_dialog: PASSED")
end

-- Run all unit tests
test_descriptor()
test_parse_url()
test_update_file_info()
test_multi_chunk_http_socket_without_misc()
test_search_by_phash_populates_mainlist()
test_download_selection()
test_help_dialog()

print(string.format("=== All %d VLC Lua Plugin Unit Tests PASSED (100%%) ===", tests_passed))
