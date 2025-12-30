export type AddonManagerEvents = {
  "scan:start": [];
  "scan:progress": [folder: string];
  "scan:complete": [count: number];
  "addon:update-check:start": [name: string];
  "addon:update-check:complete": [
    name: string,
    updateAvailable: boolean,
    remoteVersion: string,
  ];
  "addon:install:start": [name: string];
  "addon:install:downloading": [name: string];
  "addon:install:extracting": [name: string];
  "addon:install:copying": [name: string];
  "addon:install:complete": [name: string];
  "addon:remove:start": [name: string];
  "addon:remove:complete": [name: string];
  "install:folder_ownership": [parentFolder: string, ownedFolders: string[]];
  error: [context: string, message: string];
};
