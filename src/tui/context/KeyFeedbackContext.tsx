import type React from "react";
import { createContext, useCallback, useContext, useState } from "react";

interface KeyFeedbackContextType {
	activeKey: string | null;
	flashKey: (key: string) => void;
}

const KeyFeedbackContext = createContext<KeyFeedbackContextType | undefined>(
	undefined,
);

export const KeyFeedbackProvider: React.FC<{ children: React.ReactNode }> = ({
	children,
}) => {
	const [activeKey, setActiveKey] = useState<string | null>(null);

	const flashKey = useCallback((key: string) => {
		setActiveKey(key);
		setTimeout(() => setActiveKey(null), 150);
	}, []);

	return (
		<KeyFeedbackContext.Provider value={{ activeKey, flashKey }}>
			{children}
		</KeyFeedbackContext.Provider>
	);
};

export const useKeyFeedback = () => {
	const context = useContext(KeyFeedbackContext);
	if (!context) {
		throw new Error("useKeyFeedback must be used within a KeyFeedbackProvider");
	}
	return context;
};
