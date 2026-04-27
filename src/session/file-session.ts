export interface OpenFileState {
  path: string;
  savedContent: string;
  draftContent: string;
  isLoading: boolean;
}

export interface FileSessionState {
  selectedPath: string | null;
  /** Insertion order of open files — used for tab layout. */
  openOrder: string[];
  /** Most-recently-used first. Drives LRU auto-close when the tab budget is exceeded. */
  accessOrder: string[];
  files: Record<string, OpenFileState>;
}

export function createEmptyFileSession(): FileSessionState {
  return {
    selectedPath: null,
    openOrder: [],
    accessOrder: [],
    files: {},
  };
}

function touchAccess(order: string[], path: string): string[] {
  const filtered = order.filter((p) => p !== path);
  return [path, ...filtered];
}

export function openFileInSession(
  state: FileSessionState,
  path: string,
  content: string,
  isLoading = false,
): FileSessionState {
  const existing = state.files[path];
  return {
    selectedPath: path,
    openOrder: existing ? state.openOrder : [...state.openOrder, path],
    accessOrder: touchAccess(state.accessOrder, path),
    files: {
      ...state.files,
      [path]: existing ?? { path, savedContent: content, draftContent: content, isLoading },
    },
  };
}

export function setOpenFileLoading(state: FileSessionState, path: string, isLoading: boolean): FileSessionState {
  const existing = state.files[path] ?? { path, savedContent: "", draftContent: "", isLoading };
  return {
    ...openFileInSession(state, path, existing.savedContent, isLoading),
    files: {
      ...state.files,
      [path]: { ...existing, isLoading },
    },
  };
}

export function setLoadedFileContent(state: FileSessionState, path: string, content: string): FileSessionState {
  const existing = state.files[path];
  const preserveDraft = !!existing && existing.draftContent !== existing.savedContent;
  return {
    ...openFileInSession(state, path, content, false),
    files: {
      ...state.files,
      [path]: {
        path,
        savedContent: content,
        draftContent: preserveDraft && existing ? existing.draftContent : content,
        isLoading: false,
      },
    },
  };
}

export function selectOpenFile(state: FileSessionState, path: string): FileSessionState {
  if (!state.files[path]) return state;
  return { ...state, selectedPath: path, accessOrder: touchAccess(state.accessOrder, path) };
}

export function closeOpenFile(state: FileSessionState, path: string): FileSessionState {
  if (!state.files[path]) return state;
  const openOrder = state.openOrder.filter((candidate) => candidate !== path);
  const accessOrder = state.accessOrder.filter((candidate) => candidate !== path);
  const files = { ...state.files };
  delete files[path];
  if (state.selectedPath !== path) {
    return { ...state, openOrder, accessOrder, files };
  }
  const closedIndex = state.openOrder.indexOf(path);
  const nextSelected = openOrder[closedIndex] ?? openOrder[closedIndex - 1] ?? null;
  return {
    selectedPath: nextSelected,
    openOrder,
    accessOrder,
    files,
  };
}

/**
 * Trims the session to at most `maxTabs` open files by closing least-recently-used
 * files that don't have unsaved changes. Returns the unchanged state if already
 * within budget or if every candidate is dirty.
 */
export function enforceOpenFileLimit(state: FileSessionState, maxTabs: number): FileSessionState {
  if (state.openOrder.length <= maxTabs) return state;
  const selected = state.selectedPath;
  // Pick candidates from the back of the access list (oldest first), skipping
  // the currently selected file and any file with unsaved changes — matches
  // IntelliJ's behaviour where dirty tabs stay pinned in place.
  const candidates = [...state.accessOrder].reverse().filter((path) => {
    if (path === selected) return false;
    const file = state.files[path];
    if (!file) return true;
    return file.draftContent === file.savedContent;
  });
  let next = state;
  for (const path of candidates) {
    if (next.openOrder.length <= maxTabs) break;
    next = closeOpenFile(next, path);
  }
  return next;
}

export function updateFileDraft(state: FileSessionState, path: string, draftContent: string): FileSessionState {
  const existing = state.files[path];
  if (!existing) return state;
  return {
    ...state,
    files: {
      ...state.files,
      [path]: { ...existing, draftContent },
    },
  };
}

export function markFileSaved(state: FileSessionState, path: string, content: string): FileSessionState {
  const existing = state.files[path];
  if (!existing) return state;
  return {
    ...state,
    files: {
      ...state.files,
      [path]: {
        ...existing,
        savedContent: content,
        draftContent: content,
        isLoading: false,
      },
    },
  };
}

export function reorderOpenFiles(state: FileSessionState, orderedPaths: string[]): FileSessionState {
  const known = new Set(state.openOrder);
  const sanitized = orderedPaths.filter((path) => known.has(path));
  if (sanitized.length !== state.openOrder.length) return state;
  return { ...state, openOrder: sanitized };
}

export function removeOpenFiles(state: FileSessionState, paths: string[]): FileSessionState {
  let next = state;
  for (const path of paths) {
    next = closeOpenFile(next, path);
  }
  return next;
}

export function renameOpenFilePaths(
  state: FileSessionState,
  renamePath: (path: string) => string | null,
): FileSessionState {
  const renamedEntries = Object.entries(state.files)
    .map(([path, file]) => {
      const nextPath = renamePath(path);
      return nextPath ? [nextPath, { ...file, path: nextPath }] as const : null;
    })
    .filter((entry): entry is readonly [string, OpenFileState] => !!entry);

  const nextFiles = Object.fromEntries(renamedEntries);
  const nextOpenOrder = state.openOrder
    .map((path) => renamePath(path))
    .filter((path): path is string => !!path);
  const nextSelectedPath = state.selectedPath ? renamePath(state.selectedPath) : null;

  const nextAccessOrder = state.accessOrder
    .map((path) => renamePath(path))
    .filter((path): path is string => !!path);

  return {
    selectedPath: nextSelectedPath,
    openOrder: nextOpenOrder,
    accessOrder: nextAccessOrder,
    files: nextFiles,
  };
}
