import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import { useQueryClient } from "@tanstack/react-query";
import { backend, type Unsubscribe } from "@/lib/backend";
import type {
  DupeGroup,
  DupeProgress,
  ScanComplete,
  ScanProgress,
  SearchResultItem,
} from "@/lib/types";

interface RunningTasks {
  // Drive scan
  scanning: boolean;
  scanProgress: ScanProgress | null;
  scanResult: ScanComplete | null;
  scanError: string | null;
  startScan: (driveLetters?: string[]) => Promise<void>;

  // Duplicate finder
  dupeScanning: boolean;
  dupeProgress: DupeProgress | null;
  dupeGroups: DupeGroup[] | null;
  setDupeGroups: Dispatch<SetStateAction<DupeGroup[] | null>>;
  runDupeScan: (roots?: string[]) => Promise<void>;

  // Deep file search
  deepQuery: string;
  deepSearching: boolean;
  deepFound: number;
  deepResults: SearchResultItem[] | null;
  setDeepResults: Dispatch<SetStateAction<SearchResultItem[] | null>>;
  runDeepSearch: (query: string) => Promise<void>;
}

const Ctx = createContext<RunningTasks | null>(null);

/** Holds the state of long-running operations at the app root so navigating
 *  between pages never cancels or loses an in-flight scan / search. */
export function RunningTasksProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();

  // ---- Drive scan ----
  const [scanning, setScanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null);
  const [scanResult, setScanResult] = useState<ScanComplete | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);

  // ---- Duplicate finder ----
  const [dupeScanning, setDupeScanning] = useState(false);
  const [dupeProgress, setDupeProgress] = useState<DupeProgress | null>(null);
  const [dupeGroups, setDupeGroups] = useState<DupeGroup[] | null>(null);

  // ---- Deep search ----
  const [deepQuery, setDeepQuery] = useState("");
  const [deepSearching, setDeepSearching] = useState(false);
  const [deepFound, setDeepFound] = useState(0);
  const [deepResults, setDeepResults] = useState<SearchResultItem[] | null>(null);

  const subs = useRef<Unsubscribe[]>([]);

  useEffect(() => {
    let disposed = false;
    Promise.all([
      backend.onScanProgress((p) => setScanProgress(p)),
      backend.onScanComplete((c) => {
        setScanResult(c);
        setScanning(false);
        setScanProgress(null);
        queryClient.invalidateQueries({ queryKey: ["lastScan"] });
        queryClient.invalidateQueries({ queryKey: ["scanComparison"] });
        queryClient.invalidateQueries({ queryKey: ["recommendations"] });
      }),
      backend.onScanError((message) => {
        setScanError(message);
        setScanning(false);
        setScanProgress(null);
      }),
      backend.onDupeProgress((p) => setDupeProgress(p)),
      backend.onSearchProgress((p) => setDeepFound(p.found)),
    ]).then((s) => {
      if (disposed) s.forEach((u) => u());
      else subs.current = s;
    });
    return () => {
      disposed = true;
      subs.current.forEach((u) => u());
      subs.current = [];
    };
  }, [queryClient]);

  const startScan = async (driveLetters: string[] = []) => {
    setScanError(null);
    setScanResult(null);
    setScanning(true);
    try {
      await backend.startScan(driveLetters);
    } catch (e) {
      setScanError(String(e));
      setScanning(false);
    }
  };

  const runDupeScan = async (roots: string[] = []) => {
    setDupeScanning(true);
    setDupeGroups(null);
    setDupeProgress(null);
    try {
      setDupeGroups(await backend.findDuplicates(roots));
    } finally {
      setDupeScanning(false);
      setDupeProgress(null);
    }
  };

  const runDeepSearch = async (query: string) => {
    setDeepQuery(query);
    setDeepSearching(true);
    setDeepFound(0);
    setDeepResults(null);
    try {
      setDeepResults(await backend.searchFiles(query));
    } catch {
      setDeepResults([]);
    } finally {
      setDeepSearching(false);
    }
  };

  return (
    <Ctx.Provider
      value={{
        scanning,
        scanProgress,
        scanResult,
        scanError,
        startScan,
        dupeScanning,
        dupeProgress,
        dupeGroups,
        setDupeGroups,
        runDupeScan,
        deepQuery,
        deepSearching,
        deepFound,
        deepResults,
        setDeepResults,
        runDeepSearch,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function useRunningTasks(): RunningTasks {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useRunningTasks must be used within RunningTasksProvider");
  return ctx;
}
