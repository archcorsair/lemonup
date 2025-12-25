# Product Guidelines

## Prose Style & Voice
- **Hybrid "WoW-Themed" Communication:** Blend clear, direct information with flavorful, game-specific references.
    - *Example:* "Update complete. Job's done!" or "Ready to work."
    - *Goal:* Maintain clarity while injecting personality that resonates with the target audience.
- **Personality:** A mix of professional reliability and playful, community-oriented passion, with a heavier emphasis on the latter. The tool should feel like it was built by a fan for fans.

## Visual Identity
- **Balanced Information Density:** Present a rich amount of data ("just right") without overwhelming the user. Avoid clutter while ensuring users don't have to "hunt" for key information.
- **Strong Visual Hierarchy:** Use color, bolding, and whitespace effectively to guide the eye to primary actions and status indicators.
- **Accessibility:** Prioritize high contrast and clear readability, ensuring compatibility with standard terminal color schemes and diverse user needs.

## Error Handling & Feedback
- **Context-Aware Strategy:**
    - **Fail Fast:** For critical, unrecoverable errors (e.g., database corruption), halt safely and provide necessary technical details.
    - **Graceful Degradation:** For non-blocking issues (e.g., one addon failing to update in a batch), skip the failure and continue processing the rest.
    - **User-Guided Resolution:** Always attempt to explain the "why" behind an error and offer actionable steps to fix it (e.g., "Check network" or "Verify URL").

## Help & Documentation
- **Multi-Layered Support:**
    - **Discoverable Shortcuts:** Maintain existing support for `?` or `h` hotkeys to trigger contextual help.
    - **External References:** Continue using brief in-tool text that links to comprehensive external documentation where appropriate.
    - **Built-in Comprehensive Help:** Expand the current system to include more detailed, searchable explanations of features and commands directly within the TUI.
