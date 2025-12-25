import { type Instance, render } from "ink";
import type { ReactNode } from "react";

type InkRender = typeof render;
type WithFullScreen = (
  node: ReactNode,
  options?: Parameters<InkRender>[1],
) => {
  instance: Instance;
  start: () => Promise<void>;
  waitUntilExit: () => Promise<void>;
};

async function write(content: string) {
  return new Promise<void>((resolve, reject) => {
    process.stdout.write(content, (error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

async function cleanUpOnExit(instance: Instance) {
  await instance.waitUntilExit();
  await write("\x1b[?1049l"); // close the alternate buffer
}

export const withFullScreen: WithFullScreen = (node, options) => {
  const instance = render(null, options);
  const exitPromise = cleanUpOnExit(instance);

  function waitUntilExit() {
    return exitPromise;
  }

  return {
    instance: instance,
    start: async () => {
      await write("\x1b[?1049h\x1b[H\x1b[2J"); // open alternate buffer, home cursor, clear screen
      instance.rerender(node);
    },
    waitUntilExit,
  };
};
