export interface OpenFileState {
  path: string;
  savedContent: string;
  draftContent: string;
  isLoading: boolean;
}

export interface FileSessionState {
  selectedPath: string | null;
  openOrder: string[];
  files: Record<string, OpenFileState>;
}

export function createEmptyFileSession(): FileSessionState {
  return {
    selectedPath: null,
    openOrder: [],
    files: {},
  };
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
  return { ...state, selectedPath: path };
}

export function closeOpenFile(state: FileSessionState, path: string): FileSessionState {
  if (!state.files[path]) return state;
  const openOrder = state.openOrder.filter((candidate) => candidate !== path);
  const files = { ...state.files };
  delete files[path];
  if (state.selectedPath !== path) {
    return { ...state, openOrder, files };
  }
  const closedIndex = state.openOrder.indexOf(path);
  const nextSelected = openOrder[closedIndex] ?? openOrder[closedIndex - 1] ?? null;
  return {
    selectedPath: nextSelected,
    openOrder,
    files,
  };
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

  return {
    selectedPath: nextSelectedPath,
    openOrder: nextOpenOrder,
    files: nextFiles,
  };
}
