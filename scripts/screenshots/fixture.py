#!/usr/bin/env python3
"""Builds the offline LemonUp screenshot sandbox.

Usage: fixture.py SANDBOX_DIR

Creates, inside SANDBOX_DIR (which must already exist and be empty):

  World of Warcraft/                fake retail install that passes validation
    Data/  .build.info
    _retail_/Wow.exe
    _retail_/Interface/AddOns/...   addon folders with plausible .toc files
    _retail_/WTF/...                a few small files
  home/.config/lemonup/config.json  LemonUp v1 config (migrated on first run)
  home/.config/lemonup/lemonup.db   LemonUp v1 tracked-addon database
  xdg/data/lemonup/backups/wtf/     pre-made WTF backup archives
  fresh/home/Games/World of Warcraft/
                                    a second fake install, with _retail_ and _forever_
                                    AddOns folders, for the first-run setup screen
                                    (fresh/ is a separate HOME with no LemonUp files)

Nothing here talks to the network. Database timestamps are relative to "now" and backup
archive names are fixed, so reruns render the same images.
"""

import json
import sqlite3
import sys
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

INTERFACE = "120005"
NOW = datetime.now(timezone.utc).replace(microsecond=0)


def ts(delta: timedelta) -> str:
    return (NOW - delta).strftime("%Y-%m-%dT%H:%M:%SZ")


def days(n: float) -> timedelta:
    return timedelta(days=n)


def hours(n: float) -> timedelta:
    return timedelta(hours=n)


# Tracked addons. `children` are owned folders that ship in the same package.
# `remote` is the last remote version LemonUp saw; when it differs from the
# installed version (or commit, for GitHub) the addon list shows "Update ready", or
# "Newer than source" when the remote version is lower than the installed one.
TRACKED = [
    {
        "folder": "WeakAuras",
        "title": "WeakAuras",
        "author": "The WeakAuras Team",
        "version": "5.20.4",
        "source": "github",
        "url": "https://github.com/WeakAuras/WeakAuras2",
        "commit": "4f1c9a7e2b8d3c6f0a5e9b1d7c2f8a4e6b0d3c9f",
        "remote": "b83e51d09c4a7f26e1d8b3a5c0f94e7d2a6b1c85",
        "children": [
            "WeakAurasArchive",
            "WeakAurasModelPaths",
            "WeakAurasOptions",
            "WeakAurasTemplates",
        ],
        "installed": days(41),
        "updated": days(12),
    },
    {
        "folder": "DBM-Core",
        "title": "Deadly Boss Mods",
        "author": "MysticalOS, QartemisT",
        "version": "12.0.8",
        "source": "wago",
        "url": "https://addons.wago.io/addons/dbm",
        "remote": "12.0.10",
        "children": [
            "DBM-Party-Midnight",
            "DBM-Raids-Midnight",
            "DBM-StatusBarTimers",
        ],
        "installed": days(63),
        "updated": days(9),
    },
    {
        "folder": "Details",
        "title": "Details! Damage Meter",
        "author": "Terciob",
        "version": "12.0.5.14390",
        "source": "wago",
        "url": "https://addons.wago.io/addons/details",
        "remote": "12.0.5.14390",
        "children": [
            "Details_DataStorage",
            "Details_EncounterDetails",
            "Details_Streamer",
            "Details_TinyThreat",
        ],
        "installed": days(88),
        "updated": days(24),
    },
    {
        "folder": "Pawn",
        "title": "Pawn",
        "author": "Vger",
        "version": "2.11.3",
        "source": "wowinterface",
        "url": "https://www.wowinterface.com/downloads/info4802-Pawn.html",
        "remote": "2.12.0",
        "children": [],
        "installed": days(120),
        "updated": days(33),
    },
    {
        "folder": "ElvUI",
        "title": "ElvUI",
        "author": "Elv, Simpy",
        "version": "14.06",
        "source": "tukui",
        "url": "https://tukui.org/elvui",
        "remote": "14.06",
        "children": ["ElvUI_Libraries", "ElvUI_Options"],
        "installed": days(150),
        "updated": days(3),
    },
    {
        "folder": "BigWigs",
        "title": "BigWigs",
        "author": "BigWigsMods",
        "version": "v412.2",
        "source": "github",
        "url": "https://github.com/BigWigsMods/BigWigs",
        "commit": "e0a4c7b19d3f58a2c6e1b9d40f7a3c85e2b6d19a",
        "remote": "e0a4c7b19d3f58a2c6e1b9d40f7a3c85e2b6d19a",
        "children": ["BigWigs_Core", "BigWigs_Options", "BigWigs_Plugins"],
        "installed": days(70),
        "updated": days(2),
    },
    {
        "folder": "Hekili",
        "title": "Hekili",
        "author": "Hekili",
        "version": "v12.0.5-1.0.2",
        "source": "github",
        "url": "https://github.com/Hekili/hekili",
        "commit": "9d2b6e1f04a8c37b5e9f1a2d6c8b0e4f7a3d5c18",
        "remote": "9d2b6e1f04a8c37b5e9f1a2d6c8b0e4f7a3d5c18",
        "children": [],
        "installed": days(30),
        "updated": days(5),
    },
    {
        "folder": "Plater",
        "title": "Plater Nameplates",
        "author": "Terciob, Ariani",
        "version": "v624-Midnight",
        "source": "wago",
        "url": "https://addons.wago.io/addons/plater",
        "remote": "v624-Midnight",
        "children": [],
        "installed": days(95),
        "updated": days(6),
    },
    {
        "folder": "Bartender4",
        "title": "Bartender4",
        "author": "Nevcairiel",
        "version": "4.16.3",
        "source": "wago",
        "url": "https://addons.wago.io/addons/bartender4",
        "remote": "4.16.3",
        "children": [],
        "installed": days(200),
        "updated": days(18),
    },
    {
        "folder": "Auctionator",
        "title": "Auctionator",
        "author": "Borjamacare, plusmouse",
        "version": "12.0.6",
        "source": "wago",
        "url": "https://addons.wago.io/addons/auctionator",
        "remote": "12.0.7",
        "children": [],
        "installed": days(180),
        "updated": days(11),
    },
    {
        "folder": "OmniCC",
        "title": "OmniCC",
        "author": "Tuller",
        "version": "12.0.2",
        "source": "wago",
        "url": "https://addons.wago.io/addons/omnicc",
        "remote": "12.0.2",
        "children": ["OmniCC_Config"],
        "installed": days(210),
        "updated": days(27),
    },
    {
        "folder": "TomTom",
        "title": "TomTom",
        "author": "Cladhaire, Ludovicus",
        "version": "v4.1.2-release",
        "source": "wowinterface",
        "url": "https://www.wowinterface.com/downloads/info7032-TomTom.html",
        "remote": "v4.1.2-release",
        "children": [],
        "installed": days(240),
        "updated": days(40),
    },
    {
        "folder": "HandyNotes",
        "title": "HandyNotes",
        "author": "Xinhuan",
        "version": "v1.6.15",
        "source": "wowinterface",
        "url": "https://www.wowinterface.com/downloads/info10305-HandyNotes.html",
        # Provider lists an older build than the one installed: "Newer than source".
        "remote": "v1.6.14",
        "children": [],
        "installed": days(240),
        "updated": days(52),
    },
    {
        "folder": "GatherMate2",
        "title": "GatherMate2",
        "author": "kagaro, xinhuan, nevcairiel",
        "version": "1.50.6",
        "source": "wowinterface",
        "url": "https://www.wowinterface.com/downloads/info12391-GatherMate2.html",
        "remote": "1.50.6",
        "children": [],
        "installed": days(240),
        "updated": days(46),
    },
]

# Present on disk but never tracked: the addon list shows these as "Not linked".
UNTRACKED = [
    {"folder": "!BugGrabber", "title": "!BugGrabber", "author": "Funkydude", "version": "v12.0.1"},
    {"folder": "BugSack", "title": "BugSack", "author": "Funkydude", "version": "v12.0.0",
     "deps": "!BugGrabber"},
    {"folder": "SimulationCraft", "title": "SimulationCraft", "author": "Theck, navv_, seriallos",
     "version": "v12.0.5-01"},
    {"folder": "MythicDungeonTools", "title": "Mythic Dungeon Tools", "author": "Nnogga",
     "version": "6.1.3"},
]


def write_toc(folder: Path, title: str, author: str, version: str, deps: str = "",
              load_on_demand: bool = False) -> None:
    folder.mkdir(parents=True, exist_ok=True)
    lines = [
        f"## Interface: {INTERFACE}",
        f"## Title: {title}",
        f"## Author: {author}",
        f"## Version: {version}",
        "## Notes: LemonUp screenshot fixture",
    ]
    if deps:
        lines.append(f"## Dependencies: {deps}")
    if load_on_demand:
        lines.append("## LoadOnDemand: 1")
    lines.append("")
    lines.append(f"{folder.name}.lua")
    (folder / f"{folder.name}.toc").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (folder / f"{folder.name}.lua").write_text(
        f"-- {folder.name}: placeholder for LemonUp screenshots\n", encoding="utf-8"
    )


def build_wow(root: Path) -> Path:
    wow = root / "World of Warcraft"
    (wow / "Data").mkdir(parents=True)
    (wow / ".build.info").write_text(
        "Branch!STRING:0|Active!DEC:1|Version!STRING:0|Product!STRING:0\n"
        "us|1|12.0.5.64012|wow\n",
        encoding="utf-8",
    )
    client = wow / "_retail_"
    client.mkdir()
    (client / "Wow.exe").write_bytes(b"MZ fixture\n")

    addons = client / "Interface" / "AddOns"
    addons.mkdir(parents=True)
    for addon in TRACKED:
        write_toc(addons / addon["folder"], addon["title"], addon["author"], addon["version"])
        for child in addon["children"]:
            write_toc(
                addons / child,
                child.replace("_", " "),
                addon["author"],
                addon["version"],
                deps=addon["folder"],
                load_on_demand=True,
            )
    for addon in UNTRACKED:
        write_toc(addons / addon["folder"], addon["title"], addon["author"], addon["version"],
                  deps=addon.get("deps", ""))

    wtf = client / "WTF"
    sv = wtf / "Account" / "LEMONUP" / "SavedVariables"
    char = wtf / "Account" / "LEMONUP" / "Silvermoon" / "Zestara"
    sv.mkdir(parents=True)
    char.mkdir(parents=True)
    (wtf / "Config.wtf").write_text('SET locale "enUS"\nSET gxWindowedResolution "2560x1440"\n',
                                    encoding="utf-8")
    (sv / "WeakAuras.lua").write_text("WeakAurasSaved = {}\n", encoding="utf-8")
    (sv / "Details.lua").write_text("_detalhes_global = {}\n", encoding="utf-8")
    (sv / "ElvUI.lua").write_text("ElvDB = {}\n", encoding="utf-8")
    (char / "layout-local.txt").write_text("", encoding="utf-8")
    (char / "AddOns.txt").write_text(
        "\n".join(f"{a['folder']}: enabled" for a in TRACKED) + "\n", encoding="utf-8"
    )
    return addons


def build_fresh_install(home: Path) -> None:
    """A second install under a separate, empty HOME, found by setup's search."""
    wow = home / "Games" / "World of Warcraft"
    (wow / "Data").mkdir(parents=True)
    (wow / ".build.info").write_text(
        "Branch!STRING:0|Active!DEC:1|Version!STRING:0|Product!STRING:0\n"
        "us|1|12.0.5.64012|wow\n",
        encoding="utf-8",
    )
    retail = wow / "_retail_"
    retail.mkdir()
    (retail / "Wow.exe").write_bytes(b"MZ fixture\n")
    addons = retail / "Interface" / "AddOns"
    for addon in TRACKED + UNTRACKED:
        write_toc(addons / addon["folder"], addon["title"], addon["author"], addon["version"])
    forever = wow / "_forever_" / "Interface" / "AddOns"
    for folder, title, author in (
        ("Questie", "Questie", "Aero, Logon, Muehe"),
        ("Details", "Details! Damage Meter", "Terciob"),
        ("AtlasLootClassic", "AtlasLoot Classic", "Lag"),
    ):
        write_toc(forever / folder, title, author, "1.0.0")


def build_v1_state(v1_dir: Path, addons_dir: Path) -> None:
    v1_dir.mkdir(parents=True)
    config = {
        "destDir": str(addons_dir),
        "userAgent": "LemonUp/1",
        "repositories": [],
        "maxConcurrent": 3,
        "nerdFonts": True,
        "checkInterval": 300000,
        "autoCheckEnabled": False,
        "autoCheckInterval": 3600000,
        "backupWTF": True,
        "backupRetention": 5,
        "debug": False,
        "showLibs": False,
        "theme": "dark",
        "terminalProgress": True,
        "lastGlobalCheck": 0,
    }
    (v1_dir / "config.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")

    db = sqlite3.connect(v1_dir / "lemonup.db")
    db.executescript(
        """
        CREATE TABLE addons (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            folder TEXT NOT NULL,
            owned_folders TEXT NOT NULL DEFAULT '[]',
            kind TEXT NOT NULL DEFAULT 'addon',
            kind_override INTEGER NOT NULL DEFAULT 0,
            flavor TEXT NOT NULL DEFAULT 'retail',
            version TEXT,
            git_commit TEXT,
            author TEXT,
            interface TEXT,
            url TEXT,
            type TEXT NOT NULL,
            required_deps TEXT NOT NULL DEFAULT '[]',
            optional_deps TEXT NOT NULL DEFAULT '[]',
            embedded_libs TEXT NOT NULL DEFAULT '[]',
            install_date TEXT NOT NULL,
            last_updated TEXT NOT NULL,
            last_checked TEXT,
            remote_version TEXT
        );
        CREATE UNIQUE INDEX idx_addons_folder ON addons(folder);
        PRAGMA user_version = 2;
        """
    )
    for i, addon in enumerate(TRACKED):
        db.execute(
            """INSERT INTO addons (name, folder, owned_folders, version, git_commit, author,
                   interface, url, type, install_date, last_updated, last_checked,
                   remote_version)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                addon["title"],
                addon["folder"],
                json.dumps(addon["children"]),
                addon["version"],
                addon.get("commit"),
                addon["author"],
                INTERFACE,
                addon["url"],
                addon["source"],
                ts(addon["installed"]),
                ts(addon["updated"]),
                ts(hours(2) + timedelta(minutes=i)),
                addon["remote"],
            ),
        )
    db.commit()
    db.close()


def build_backups(backup_dir: Path, wtf_dir: Path) -> None:
    backup_dir.mkdir(parents=True)
    # Fixed stamps: archive names are on screen, so they stay byte-stable.
    for stamp in ("20261114T091502417Z", "20261111T203318052Z", "20261105T174240913Z"):
        archive = backup_dir / f"WTF-{stamp}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
            for path in sorted(wtf_dir.rglob("*")):
                arcname = "WTF/" + path.relative_to(wtf_dir).as_posix()
                if path.is_dir():
                    zf.writestr(arcname + "/", "")
                else:
                    zf.write(path, arcname)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: fixture.py SANDBOX_DIR", file=sys.stderr)
        return 2
    root = Path(sys.argv[1])
    if not root.is_dir() or any(root.iterdir()):
        print(f"sandbox must be an existing empty directory: {root}", file=sys.stderr)
        return 2

    addons = build_wow(root)
    build_v1_state(root / "home" / ".config" / "lemonup", addons)
    build_backups(
        root / "xdg" / "data" / "lemonup" / "backups" / "wtf",
        addons.parent.parent / "WTF",
    )
    build_fresh_install(root / "fresh" / "home")
    folders = sorted(p.name for p in addons.iterdir())
    print(f"fixture: {len(folders)} addon folders, {len(TRACKED)} tracked, "
          f"{sum(1 for a in TRACKED if a['remote'] != a.get('commit', a['version']))} "
          "with a different remote version")
    return 0


if __name__ == "__main__":
    sys.exit(main())
