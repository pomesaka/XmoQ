// What Chromium has of the File System Access API (WICG,
// https://wicg.github.io/file-system-access/) beyond the DOM library of
// TypeScript, which has the handles but not the picker nor the permissions.
// WHY optional: a browser without them (Firefox, Safari) has none, and a
// caller looks before it calls.

interface FileSystemHandlePermissionDescriptor {
	mode?: "read" | "readwrite";
}

interface FileSystemHandle {
	queryPermission?(
		descriptor?: FileSystemHandlePermissionDescriptor,
	): Promise<PermissionState>;
	requestPermission?(
		descriptor?: FileSystemHandlePermissionDescriptor,
	): Promise<PermissionState>;
}

interface DirectoryPickerOptions {
	id?: string;
	mode?: "read" | "readwrite";
	startIn?: "desktop" | "documents" | "downloads" | FileSystemHandle;
}

interface Window {
	showDirectoryPicker?(
		options?: DirectoryPickerOptions,
	): Promise<FileSystemDirectoryHandle>;
}
