import { Database } from "bun:sqlite";
import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseManager } from "@/core/db";

describe("Database Migration V1 to V2", () => {
  let tempDir: string;
  let dbPath: string;

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "lemonup-db-test-"));
    dbPath = path.join(tempDir, "lemonup.db");
  });

  afterEach(() => {
    fs.rmSync(tempDir, { recursive: true, force: true });
  });

  it("should migrate V1 database to V2", () => {
    // 1. Create V1 Database manually
    const v1db = new Database(dbPath);
    v1db.run(`
      CREATE TABLE IF NOT EXISTS addons (
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
        last_updated TEXT NOT NULL
      );
    `);
    v1db.run("PRAGMA user_version = 1;");
    v1db.run(`
      INSERT INTO addons (name, folder, type, install_date, last_updated)
      VALUES ('TestAddon', 'TestAddon', 'manual', '2024-01-01', '2024-01-01')
    `);
    v1db.close();

    // 2. Initialize DatabaseManager (should trigger migration)
    const manager = new DatabaseManager(tempDir);

    // 3. Verify Schema V2
    const v2db = new Database(dbPath);
    const columns = v2db.query("PRAGMA table_info(addons)").all() as any[];
    const hasLastChecked = columns.some((c) => c.name === "last_checked");
    const hasRemoteVersion = columns.some((c) => c.name === "remote_version");
    const version = (v2db.query("PRAGMA user_version").get() as any)
      .user_version;

    expect(hasLastChecked).toBe(true);
    expect(hasRemoteVersion).toBe(true);
    expect(version).toBe(2);

    // 4. Verify Data Preservation
    const addon = manager.getByFolder("TestAddon");
    expect(addon).not.toBeNull();
    expect(addon?.name).toBe("TestAddon");
    expect(addon?.last_checked).toBeNull(); // Default
    expect(addon?.remote_version).toBeNull(); // Default

    manager.close();
    v2db.close();
  });

  it("should create V2 schema for fresh install", () => {
    const manager = new DatabaseManager(tempDir);
    const db = new Database(dbPath);
    
    const version = (db.query("PRAGMA user_version").get() as any).user_version;
    const columns = db.query("PRAGMA table_info(addons)").all() as any[];
    const hasLastChecked = columns.some((c) => c.name === "last_checked");

    expect(version).toBe(2);
    expect(hasLastChecked).toBe(true);

    manager.close();
    db.close();
  });
});
