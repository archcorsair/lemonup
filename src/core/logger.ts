import fs from "node:fs/promises";
import path from "node:path";

export class Logger {
  private static instance: Logger;
  private logPath: string;
  private enabled: boolean = false;

  private constructor() {
    // Defaults to current working directory for debug.log
    this.logPath = path.join(process.cwd(), "debug.log");
  }

  public static getInstance(): Logger {
    if (!Logger.instance) {
      Logger.instance = new Logger();
    }
    return Logger.instance;
  }

  public setEnabled(enabled: boolean) {
    this.enabled = enabled;
  }

  public setLogPath(logPath: string) {
    this.logPath = logPath;
  }

  public getLogPath(): string {
    return this.logPath;
  }

  private getTimestamp(): string {
    return new Date().toISOString();
  }

  public async log(category: string, message: string) {
    if (!this.enabled) return;

    const logMessage = `[${this.getTimestamp()}] [${category}] ${message}\n`;
    try {
      await fs.appendFile(this.logPath, logMessage);
    } catch {
      // Maybe console.error if we are in a context where that works?
      // In TUI mode console.error might break layout, best to ignore for now.
    }
  }

  public async error(category: string, message: string, error?: unknown) {
    if (!this.enabled) return;

    const errorMessage = error
      ? `: ${error instanceof Error ? error.stack || error.message : String(error)}`
      : "";
    const logMessage = `[${this.getTimestamp()}] [${category}] [ERROR] ${message}${errorMessage}\n`;

    try {
      await fs.appendFile(this.logPath, logMessage);
    } catch {
      // Ignore write errors
    }
  }
}

export const logger = Logger.getInstance();
