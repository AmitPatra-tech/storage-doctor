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
import { refreshAfterCleanup } from "@/lib/refresh";
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

  // Search box — held here, not in the page, so switching tabs mid-search
  // does not wipe the query the running search belongs to.
  searchInput: string;
  setSearchInput: Dispatch<SetStateAction<string>>;
  /** `searchInput` debounced; what the indexed search actually runs on. */
  searchQuery: string;

  // Deep file search
  deepQuery: string;
  deepSearching: boolean;
  deepFound: number;
  deepResults: SearchResultItem[] | null;
  setDeepResults: Dispatch<SetStateAction<SearchResultItem[] | null>>;
  runDeepSearch: (query: string) => Promise<void>;
  cancelDeepSearch: () => void;
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

  // ---- Search ----
  const [searchInput, setSearchInput] = useState("");
  const [searchQuery, setSearchQuery] = useState("");

  useEffect(() => {
    const t = setTimeout(() => setSearchQuery(searchInput.trim()), 300);
    return () => clearTimeout(t);
  }, [searchInput]);

  // ---- Deep search ----
  const [deepQuery, setDeepQuery] = useState("");
  const [deepSearching, setDeepSearching] = useState(false);
  const [deepFound, setDeepFound] = useState(0);
  const [deepResults, setDeepResults] = useState<SearchResultItem[] | null>(null);
  /** The query currently accepting streamed results. Held in a ref so the
   *  event listener, registered once, always sees the live value. */
  const activeDeepQuery = useRef<string | null>(null);

  const subs = useRef<Unsubscribe[]>([]);

  useEffect(() => {
    let disposed = false;
    Promise.all([
      backend.onScanProgress((p) => setScanProgress(p)),
      backend.onScanComplete((c) => {
        setScanResult(c);
        setScanning(false);
        setScanProgress(null);
        refreshAfterCleanup(queryClient);
      }),
      backend.onScanError((message) => {
        setScanError(message);
        setScanning(false);
        setScanProgress(null);
      }),
      backend.onDupeProgress((p) => setDupeProgress(p)),
      // Matches stream in while the drives are still being walked, so results
      // appear straight away instead of after the whole walk finishes.
      backend.onSearchProgress((p) => {
        if (p.query !== activeDeepQuery.current) return;
        setDeepFound(p.found);
        if (p.items.length === 0) return;
        setDeepResults((prev) => {
          const seen = new Set((prev ?? []).map((r) => r.path));
          const fresh = p.items.filter((r) => !seen.has(r.path));
          return fresh.length > 0 ? [...(prev ?? []), ...fresh] : prev;
        });
      }),
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
    // Stop whatever walk is still running before starting another, so two
    // searches never compete for the same disks.
    await backend.cancelSearch();
    activeDeepQuery.current = query;
    setDeepQuery(query);
    setDeepSearching(true);
    setDeepFound(0);
    setDeepResults([]);
    try {
      const final = await backend.searchFiles(query);
      // A newer search may have taken over while this one finished.
      if (activeDeepQuery.current === query) setDeepResults(final);
    } catch {
      if (activeDeepQuery.current === query) setDeepResults([]);
    } finally {
      if (activeDeepQuery.current === query) setDeepSearching(false);
    }
  };

  /** Stops the walk but keeps whatever it found so far on screen. */
  const cancelDeepSearch = () => {
    backend.cancelSearch();
    setDeepSearching(false);
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
        searchInput,
        setSearchInput,
        searchQuery,
        deepQuery,
        deepSearching,
        deepFound,
        deepResults,
        setDeepResults,
        runDeepSearch,
        cancelDeepSearch,
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
