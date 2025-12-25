import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import Spinner from "ink-spinner";
import type React from "react";
import type { AddonRecord } from "@/core/db";
import type { UpdateResult } from "@/core/manager";
import { THEME } from "../theme";

export type RepoStatus =
  | "idle"
  | "checking"
  | "downloading"
  | "extracting"
  | "copying"
  | "done"
  | "error";

interface RepositoryRowProps {
  repo: AddonRecord;
  status: RepoStatus;
  result?: UpdateResult;
  nerdFonts?: boolean;
  isSelected?: boolean;
  isChecked?: boolean;
  isChild?: boolean;
  isLastChild?: boolean;
}

export const RepositoryRow: React.FC<RepositoryRowProps> = ({
  repo,
  status,
  result,
  nerdFonts = true,
  isSelected = false,
  isChecked = false,
  isChild = false,
  isLastChild = false,
}) => {
  let icon = (
    <Color styles={THEME.statusIdle}>
      <Text>·</Text>
    </Color>
  );
  let statusText = (
    <Color styles={THEME.statusIdle}>
      <Text>Waiting</Text>
    </Color>
  );

  const typeLabel =
    repo.type === "tukui" ? (
      <Color styles={THEME.repoTukui}>
        <Text>[TukUI]</Text>
      </Color>
    ) : repo.type === "wowinterface" ? (
      <Color styles={THEME.repoWowi}>
        <Text>[WoWI]</Text>
      </Color>
    ) : repo.type === "manual" ? (
      <Color styles={THEME.repoManual}>
        <Text>[Man]</Text>
      </Color>
    ) : (
      <Color styles={THEME.repoGit}>
        <Text>[Git]</Text>
      </Color>
    );

  switch (status) {
    case "idle":
      icon = (
        <Color styles={THEME.statusIdle}>
          <Text>·</Text>
        </Color>
      );
      statusText = (
        <Color styles={THEME.statusIdle}>
          <Text>Idle</Text>
        </Color>
      );
      break;
    case "checking":
      icon = nerdFonts ? (
        <Color styles={THEME.statusChecking}>
          <Text>
            <SpinnerFixed type="dots" />
          </Text>
        </Color>
      ) : (
        <Color styles={THEME.statusChecking}>
          <Text>?</Text>
        </Color>
      );
      statusText = (
        <Color styles={THEME.statusChecking}>
          <Text wrap="truncate-end">Checking...</Text>
        </Color>
      );
      break;
    case "downloading":
      icon = nerdFonts ? (
        <Color styles={THEME.statusWorking}>
          <Text>
            <SpinnerFixed type="dots" />
          </Text>
        </Color>
      ) : (
        <Color styles={THEME.statusWorking}>
          <Text>↓</Text>
        </Color>
      );
      if (repo.type === "tukui" || repo.type === "wowinterface") {
        statusText = (
          <Color styles={THEME.statusWorking}>
            <Text wrap="truncate-end">Downloading Zip...</Text>
          </Color>
        );
      } else {
        statusText = (
          <Color styles={THEME.statusWorking}>
            <Text wrap="truncate-end">Git Syncing...</Text>
          </Color>
        );
      }
      break;
    case "extracting":
      icon = nerdFonts ? (
        <Color styles={THEME.statusWorking}>
          <Text>
            <SpinnerFixed type="dots" />
          </Text>
        </Color>
      ) : (
        <Color styles={THEME.statusWorking}>
          <Text>E</Text>
        </Color>
      );
      statusText = (
        <Color styles={THEME.statusWorking}>
          <Text wrap="truncate-end">Extracting...</Text>
        </Color>
      );
      break;
    case "copying":
      icon = nerdFonts ? (
        <Color styles={THEME.statusWorking}>
          <Text>
            <SpinnerFixed type="dots" />
          </Text>
        </Color>
      ) : (
        <Color styles={THEME.statusWorking}>
          <Text>C</Text>
        </Color>
      );
      statusText = (
        <Color styles={THEME.statusWorking}>
          <Text wrap="truncate-end">Copying...</Text>
        </Color>
      );
      break;
    case "done":
      if (result?.updated) {
        // Distinguish between "Update Available" (ManageScreen) and "Updated" (UpdateScreen)
        // ManageScreen uses "Update: ..." message convention
        const isUpdateAvailableMsg = result.message?.startsWith("Update:");

        if (isUpdateAvailableMsg) {
          icon = (
            <Color styles={THEME.statusWarning}>
              <Text>{nerdFonts ? "📦" : "!"}</Text>
            </Color>
          );
          statusText = (
            <Color styles={THEME.statusWarning}>
              <Text wrap="truncate-end">{result.message}</Text>
            </Color>
          );
        } else {
          // "Updated to ..." - Success
          icon = (
            <Color styles={THEME.statusSuccess}>
              <Text>{nerdFonts ? "✔" : "OK"}</Text>
            </Color>
          );
          statusText = (
            <Color styles={THEME.statusSuccess}>
              <Text wrap="truncate-end">{result.message}</Text>
            </Color>
          );
        }
      } else {
        icon = <Text> </Text>;
        statusText = (
          <Color styles={THEME.statusSuccess}>
            <Text wrap="truncate-end">Up to date</Text>
          </Color>
        );
      }
      break;
    case "error":
      icon = (
        <Color styles={THEME.statusError}>
          <Text>{nerdFonts ? "✘" : "X"}</Text>
        </Color>
      );
      statusText = (
        <Color styles={THEME.statusError}>
          <Text wrap="truncate-end">{result?.error || "Error"}</Text>
        </Color>
      );
      break;
  }

  const displayVersion = repo.version
    ? repo.version.match(/^[a-f0-9]{40}$/i)
      ? repo.version.substring(0, 7)
      : repo.version
    : "";

  // Tree View Indentation
  let namePrefix = null;
  if (isChild) {
    namePrefix = (
      <Color styles={THEME.treePrefix}>
        <Text>{isLastChild ? "└── " : "├── "}</Text>
      </Color>
    );
  }

  // Row selection/checked styles
  const nameStyle = isSelected
    ? THEME.selection
    : isChecked
      ? THEME.checked
      : undefined;

  return (
    <Box paddingX={2} width="100%">
      <Box width={3} flexShrink={0}>
        <Color styles={THEME.selection}>
          <Text>{isSelected ? ">" : " "}</Text>
        </Color>
        <Color styles={isChecked ? THEME.checked : THEME.unchecked}>
          <Text>{isChecked ? (nerdFonts ? "●" : "*") : " "}</Text>
        </Color>
      </Box>

      <Box width={22} flexShrink={0}>
        {!isChild ? (
          <Box gap={1} flexDirection="row">
            <Box flexGrow={1} flexShrink={1}>
              {statusText}
            </Box>
            <Box width={2} justifyContent="flex-end">
              {icon}
            </Box>
          </Box>
        ) : (
          <Text> </Text>
        )}
      </Box>

      <Box flexGrow={2} flexShrink={1} minWidth={15} flexBasis="20%">
        {namePrefix}
        <Color styles={nameStyle}>
          <Text wrap="truncate-end">{repo.name} </Text>
        </Color>
        {repo.kind === "library" && (
          <Color styles={THEME.library}>
            <Text>[Lib{repo.kindOverride ? "*" : ""}]</Text>
          </Color>
        )}
        {displayVersion ? (
          <Color styles={THEME.version}>
            <Text>({displayVersion})</Text>
          </Color>
        ) : null}
      </Box>

      <Box flexGrow={1} flexShrink={1} minWidth={10} flexBasis="15%">
        <Text wrap="truncate-end">{repo.author || "-"}</Text>
      </Box>

      <Box width={8} flexShrink={0}>
        {typeLabel}
      </Box>
    </Box>
  );
};

// Workaround for React 19 + Ink type mismatch
const SpinnerFixed = Spinner as unknown as React.FC<{
  type?: string;
}>;
