import { Box, Text } from "ink";
import type React from "react";

interface Shortcut {
	key: string;
	label: string;
}

interface ShortcutsModalProps {
	shortcuts: Shortcut[];
	visible: boolean;
}

export const ShortcutsModal: React.FC<ShortcutsModalProps> = ({
	shortcuts,
	visible,
}) => {
	if (!visible) return null;

	const maxKeyLen = Math.max(...shortcuts.map((s) => s.key.length));
	const maxLabelLen = Math.max(...shortcuts.map((s) => s.label.length));

	const gap = 2;
	const sidePad = 1;
	const keyColWidth = sidePad + maxKeyLen + gap;
	const labelColWidth = maxLabelLen + sidePad;
	const totalWidth = keyColWidth + labelColWidth;

	return (
		<Box
			position="absolute"
			width="100%"
			height="100%"
			justifyContent="center"
			alignItems="center"
		>
			<Box flexDirection="column" borderStyle="single" borderColor="cyan">
				<Box width={totalWidth}>
					<Text bold color="cyan" underline backgroundColor="black">
						{(" ".repeat(sidePad) + "Commands").padEnd(totalWidth)}
					</Text>
				</Box>

				<Box width={totalWidth}>
					<Text backgroundColor="black">{" ".repeat(totalWidth)}</Text>
				</Box>

				<Box flexDirection="column">
					{shortcuts.map((s) => (
						<Box key={s.key} width={totalWidth}>
							<Text color="yellow" bold backgroundColor="black">
								{(" ".repeat(sidePad) + s.key).padEnd(keyColWidth)}
							</Text>
							<Text backgroundColor="black">
								{s.label.padEnd(labelColWidth)}
							</Text>
						</Box>
					))}
				</Box>

				<Box width={totalWidth}>
					<Text backgroundColor="black">{" ".repeat(totalWidth)}</Text>
				</Box>

				<Box width={totalWidth}>
					<Text color="white" italic backgroundColor="black">
						{`${" ".repeat(sidePad)}Press key to execute`.padEnd(totalWidth)}
					</Text>
				</Box>
			</Box>
		</Box>
	);
};
