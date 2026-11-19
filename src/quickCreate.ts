// Only the native --create-task overlay uses this separate browser window.
export const QUICK_CREATE_WINDOW = new URLSearchParams(window.location.search).get("quick-create") === "1";
