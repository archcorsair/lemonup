import { Box, Text } from "ink";
import type React from "react";

interface ScreenTitleProps {
	title: string;
	children?: React.ReactNode;
}

export const ScreenTitle: React.FC<ScreenTitleProps> = ({
	title,
	children,
}) => (
	<Box flexDirection="row" gap={2}>
		<Text color="magenta" bold>
			{title}
		</Text>
		{children}
	</Box>
);
