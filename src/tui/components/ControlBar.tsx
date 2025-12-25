import { Box, Text } from "ink";
import React from "react";
import { useAppStore } from "@/tui/store/useAppStore";

export interface ControlHelp {
	key: string;
	label: string;
}

export interface ControlBarProps {
	message?: React.ReactNode;
	controls: ControlHelp[];
}

export const ControlBar: React.FC<ControlBarProps> = ({
	message,
	controls,
}) => {
	const activeKey = useAppStore((state) => state.activeKey);

	return (
		<Box marginTop={1} borderStyle="double" borderColor="gray" paddingX={1}>
			<Box flexGrow={1}>{message}</Box>
			<Box flexWrap="wrap">
				<Text>controls: </Text>
				{controls.map((ctrl, idx) => {
					const isActive =
						activeKey === ctrl.key ||
						(ctrl.key === "↑/↓" &&
							(activeKey === "up" || activeKey === "down"));
					return (
						<React.Fragment key={ctrl.key}>
							{idx > 0 && <Text>, </Text>}
							<Text bold color={isActive ? "magenta" : "white"}>
								{ctrl.key}
							</Text>
							<Text color={isActive ? "magenta" : undefined}>
								{" "}
								({ctrl.label})
							</Text>
						</React.Fragment>
					);
				})}
			</Box>
		</Box>
	);
};
