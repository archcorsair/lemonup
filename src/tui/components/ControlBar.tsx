import { Box, Text } from "ink";
import React from "react";

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
	return (
		<Box marginTop={1} borderStyle="double" borderColor="gray" paddingX={1}>
			<Box flexGrow={1}>{message}</Box>
			<Box>
				<Text>controls: </Text>
				{controls.map((ctrl, idx) => (
					<React.Fragment key={ctrl.key}>
						{idx > 0 && <Text>, </Text>}
						<Text bold color="white">
							{ctrl.key}
						</Text>
						<Text> ({ctrl.label})</Text>
					</React.Fragment>
				))}
			</Box>
		</Box>
	);
};
