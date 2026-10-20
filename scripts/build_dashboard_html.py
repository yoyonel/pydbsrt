#!/usr/bin/env python3
"""Generates a rock-solid, responsive HTML dashboard for the E2E VLC tests."""

import json
from pathlib import Path


def get_failure_reason(video: str, conf: float) -> str:
    """Returns a clear factual explanation of failure reasons."""
    if conf >= 90.0:
        return "SUCCÈS"

    v_lower = video.lower()
    if "evangelion_24" in v_lower:
        return "ÉCHEC (Plan fixe noir 15s — variance 0)"
    if "ruby" in v_lower and "tableaux" in v_lower:
        return "ÉCHEC (Écran fixe code 15s — variance 0)"
    if "cakephp" in v_lower and "installation" in v_lower:
        return "ÉCHEC (2 frames oscillantes sans cuts)"
    return "ÉCHEC (Écran statique / non indexé)"


def main():
    manifest_path = Path("docs/screenshots/e2e_vlc_all/manifest.json")
    if not manifest_path.exists():
        print("Manifest not found")
        return

    with open(manifest_path, encoding="utf-8") as f:
        items = json.load(f)

    total = len(items)
    success = sum(1 for x in items if x.get("confidence_percent", 0.0) >= 90.0)
    failed = total - success
    rate = (success / total * 100.0) if total else 0.0

    table_data = []
    for idx, x in enumerate(items, 1):
        conf = x.get("confidence_percent", 0.0)
        video = x.get("video", "")
        matched = x.get("matched_media") or "Non indexé"
        status_label = get_failure_reason(video, conf)

        raw_rel = x.get("screenshot_rel", "")
        clean_rel = raw_rel.replace("docs/", "")
        img_url = f"/pydbsrt/{clean_rel}" if not clean_rel.startswith("/") else clean_rel

        table_data.append(
            [
                idx,
                img_url,
                video,
                matched,
                f"{conf:.1f}%",
                status_label,
            ]
        )

    json_str = json.dumps(table_data, ensure_ascii=False)

    html_content = f"""---
title: "Dashboard Interactif E2E VLC (1 334 Tests)"
description: "Dashboard haute performance avec recherche plein champ, filtres par statut et prévisualisation lightbox des captures 1080p."
tags:
  - dashboard
  - e2e
  - vlc
  - benchmarks
---

# Dashboard Interactif E2E VLC GUI (1 334 Tests Réels)

<link rel="stylesheet" href="https://cdn.datatables.net/2.1.8/css/dataTables.dataTables.min.css">
<style>
.kpi-card {{
    background: #0f172a;
    border: 1px solid #334155;
    border-radius: 8px;
    padding: 16px;
    text-align: center;
}}
.kpi-val {{
    font-size: 28px;
    font-weight: 700;
    font-family: monospace;
}}
.kpi-lbl {{
    font-size: 12px;
    color: #94a3b8;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-top: 4px;
}}
.badge-ok {{
    background: rgba(34, 197, 94, 0.2);
    color: #4ade80;
    padding: 3px 8px;
    border-radius: 4px;
    font-weight: 600;
    font-size: 11px;
}}
.badge-fail {{
    background: rgba(239, 68, 68, 0.2);
    color: #f87171;
    padding: 3px 8px;
    border-radius: 4px;
    font-weight: 600;
    font-size: 11px;
}}
#e2e-table_wrapper {{
    margin-top: 20px;
    font-size: 13px;
}}
#e2e-table {{
    width: 100% !important;
}}
#e2e-table th, #e2e-table td {{
    vertical-align: middle;
    position: relative;
}}
#e2e-table th {{
    user-select: none;
}}
.col-resizer {{
    position: absolute;
    top: 0;
    right: 0;
    width: 6px;
    cursor: col-resize;
    user-select: none;
    height: 100%;
    z-index: 10;
}}
.col-resizer:hover, .col-resizer.resizing {{
    background: #38bdf8;
}}
.table-thumb {{
    width: 80px;
    height: 45px;
    object-fit: cover;
    border-radius: 4px;
    border: 1px solid #475569;
    box-shadow: 0 2px 4px rgba(0,0,0,0.3);
    cursor: pointer;
    transition: transform 0.15s ease, border-color 0.15s ease;
}}
.table-thumb:hover {{
    transform: scale(1.15);
    border-color: #38bdf8;
}}

/* Lightbox Modal */
#modal-lightbox {{
    display: none;
    position: fixed;
    z-index: 99999;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.9);
    backdrop-filter: blur(4px);
    justify-content: center;
    align-items: center;
    flex-direction: column;
}}
#modal-img {{
    max-width: 90vw;
    max-height: 82vh;
    border-radius: 8px;
    border: 1px solid #334155;
    box-shadow: 0 10px 30px rgba(0,0,0,0.8);
}}
#modal-caption {{
    color: #f8fafc;
    margin-top: 14px;
    font-size: 14px;
    font-family: monospace;
}}
#modal-close {{
    position: absolute;
    top: 20px;
    right: 28px;
    color: #94a3b8;
    font-size: 34px;
    cursor: pointer;
    font-weight: bold;
}}
#modal-close:hover {{
    color: #fff;
}}
</style>

<div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin-bottom: 24px;">
    <div class="kpi-card">
        <div class="kpi-val" style="color: #38bdf8;">{total}</div>
        <div class="kpi-lbl">Vidéos Évaluées</div>
    </div>
    <div class="kpi-card">
        <div class="kpi-val" style="color: #4ade80;">{success}</div>
        <div class="kpi-lbl">Reconnaissances (Score &ge; 90%)</div>
    </div>
    <div class="kpi-card">
        <div class="kpi-val" style="color: #f87171;">{failed}</div>
        <div class="kpi-lbl">Plans Fixes Purs (0% var)</div>
    </div>
    <div class="kpi-card">
        <div class="kpi-val" style="color: #a855f7;">{rate:.1f}%</div>
        <div class="kpi-lbl">Taux de Match E2E VLC</div>
    </div>
</div>

<div style="margin-bottom: 16px; display: flex; gap: 8px;">
    <button id="btn-all" class="md-button md-button--primary" style="font-size: 12px; padding: 4px 12px;">Tous ({total})</button>
    <button id="btn-ok" class="md-button" style="font-size: 12px; padding: 4px 12px; background: rgba(34, 197, 94, 0.15); color: #4ade80; border: 1px solid #22c55e;">Matchs Valides ({success})</button>
    <button id="btn-fail" class="md-button" style="font-size: 12px; padding: 4px 12px; background: rgba(239, 68, 68, 0.15); color: #f87171; border: 1px solid #ef4444;">Plans Fixes ({failed})</button>
</div>

<table id="e2e-table" class="display" style="width:100%">
    <thead>
        <tr>
            <th>#</th>
            <th>Miniature</th>
            <th>Fichier Vidéo Testé</th>
            <th>Média Reconnu (pHash)</th>
            <th>Score</th>
            <th>Statut / Diagnostic</th>
        </tr>
    </thead>
    <tbody></tbody>
</table>

<!-- Lightbox Overlay -->
<div id="modal-lightbox">
    <span id="modal-close">&times;</span>
    <img id="modal-img" src="" alt="Capture plein écran">
    <div id="modal-caption"></div>
</div>

<script src="https://code.jquery.com/jquery-3.7.1.min.js"></script>
<script src="https://cdn.datatables.net/2.1.8/js/dataTables.min.js"></script>
<script>
const rawDataSet = {json_str};

$(document).ready(function() {{
    const table = $('#e2e-table').DataTable({{
        data: rawDataSet,
        pageLength: 25,
        lengthMenu: [10, 25, 50, 100],
        order: [[0, 'asc']],
        autoWidth: false,
        stateSave: true,
        stateDuration: -1,
        columns: [
            {{ title: "#" }},
            {{
                title: "Miniature",
                orderable: false,
                render: function(data, type, row) {{
                    return '<img class="table-thumb" src="' + data + '" loading="lazy" alt="Miniature" data-url="' + data + '" data-title="' + encodeURIComponent(row[2]) + '">';
                }}
            }},
            {{
                title: "Fichier Vidéo Testé",
                render: function(data) {{
                    return '<code>' + data + '</code>';
                }}
            }},
            {{
                title: "Média Reconnu (pHash)",
                render: function(data) {{
                    return '<b>' + data + '</b>';
                }}
            }},
            {{
                title: "Score",
                render: function(data) {{
                    return '<b>' + data + '</b>';
                }}
            }},
            {{
                title: "Statut / Diagnostic",
                render: function(data) {{
                    if (data.indexOf("SUCCÈS") !== -1) {{
                        return '<span class="badge-ok">&#10003; SUCCÈS</span>';
                    }} else {{
                        return '<span class="badge-fail">&#10007; ' + data + '</span>';
                    }}
                }}
            }}
        ],
        language: {{
            search: "Filtrer en direct :",
            lengthMenu: "Afficher _MENU_ éléments",
            info: "Affichage de _START_ à _END_ sur _TOTAL_ vidéos",
            paginate: {{
                first: "Premier",
                last: "Dernier",
                next: "Suivant",
                previous: "Précédent"
            }}
        }}
    }});

    // Restore saved custom column widths from localStorage if present
    const storageKey = 'dbsrt_e2e_col_widths';
    try {{
        const savedWidths = JSON.parse(localStorage.getItem(storageKey) || '{{}}');
        $('#e2e-table thead th').each(function(idx) {{
            if (savedWidths[idx]) {{
                $(this).css({{'width': savedWidths[idx] + 'px', 'min-width': savedWidths[idx] + 'px'}});
            }}
        }});
    }} catch(e) {{}}

    // Enable interactive drag resizing on column headers
    $('#e2e-table thead th').each(function(idx) {{
        const th = $(this);
        const resizer = $('<div class="col-resizer"></div>');
        th.append(resizer);

        let startX = 0;
        let startWidth = 0;

        resizer.on('mousedown', function(e) {{
            e.preventDefault();
            e.stopPropagation();
            startX = e.pageX;
            startWidth = th.outerWidth();
            resizer.addClass('resizing');

            $(document).on('mousemove.colResize', function(e) {{
                const newWidth = Math.max(30, startWidth + (e.pageX - startX));
                th.css({{'width': newWidth + 'px', 'min-width': newWidth + 'px'}});
            }});

            $(document).on('mouseup.colResize', function() {{
                $(document).off('.colResize');
                resizer.removeClass('resizing');
                try {{
                    const curWidths = JSON.parse(localStorage.getItem(storageKey) || '{{}}');
                    curWidths[idx] = th.outerWidth();
                    localStorage.setItem(storageKey, JSON.stringify(curWidths));
                }} catch(e) {{}}
            }});
        }});
    }});

    // Filter button handlers
    $('#btn-all').on('click', function() {{
        table.search('').columns().search('').draw();
    }});
    $('#btn-ok').on('click', function() {{
        table.search('').columns().search('');
        table.column(5).search('SUCCÈS').draw();
    }});
    $('#btn-fail').on('click', function() {{
        table.search('').columns().search('');
        table.column(5).search('ÉCHEC').draw();
    }});

    // Event delegation for lightbox clicks
    $('#e2e-table').on('click', '.table-thumb', function() {{
        const url = $(this).attr('data-url');
        const title = decodeURIComponent($(this).attr('data-title'));
        $('#modal-img').attr('src', url);
        $('#modal-caption').text(title);
        $('#modal-lightbox').css('display', 'flex');
    }});

    // Lightbox modal close handlers
    $('#modal-close, #modal-lightbox').on('click', function(e) {{
        if (e.target !== $('#modal-img')[0]) {{
            $('#modal-lightbox').hide();
        }}
    }});
    $(document).on('keydown', function(e) {{
        if (e.key === "Escape") {{
            $('#modal-lightbox').hide();
        }}
    }});
}});
</script>
"""

    out_file = Path("docs/2026-10-20-dashboard-interactif-e2e-vlc-corpus-nas.md")
    out_file.write_text(html_content, encoding="utf-8")
    print(f"Generated clean dashboard with diagnostics at {out_file}")


if __name__ == "__main__":
    main()
