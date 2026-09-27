import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
    AlertDialog,
    AlertDialogAction,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { ChevronDown, TriangleAlert } from "lucide-react";
import { useState, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ユーザー辞書の登録候補（#17）。会話ログと Vault のノートから、よく使うのに変換で出にくい語を探して出す。
// 登録するのは、画面でチェックして確認ダイアログで承認した語だけ

export type Entry = { reading: string; word: string };

type Example = { source: "conversation" | "note"; text: string };
type Suggestion = { word: string; reading: string; count: number; examples: Example[] };
type Phase = "connect" | "read" | "check" | "reading";
type Progress = { phase: Phase; done: number; total: number; found: number };
type ScanStats = { texts: number; checked: number; llm: number; seconds: number; model: string };
type ScanResult = { stopped: boolean; suggestions: Suggestion[]; stats: ScanStats };
type Finished = { result: ScanResult | null; error: string | null };

type ScanState =
    | { status: "idle" }
    | { status: "running"; progress: Progress | null; stopping: boolean }
    | { status: "error"; error: string }
    | { status: "done"; stopped: boolean; stats: ScanStats; suggestions: Suggestion[] };

// 探している途中で別のページへ移っても、戻ったときに進み具合と結果が残るよう、状態はページの外に置く
let state: ScanState = { status: "idle" };
const subscribers = new Set<() => void>();
const setState = (next: ScanState) => {
    state = next;
    subscribers.forEach((notify) => notify());
};
const subscribe = (notify: () => void) => {
    subscribers.add(notify);
    return () => subscribers.delete(notify);
};

let listening = false;
const listenOnce = () => {
    if (listening) return;
    listening = true;
    listen<Progress>("suggestion-progress", (event) => {
        if (state.status === "running") setState({ ...state, progress: event.payload });
    });
    listen<Finished>("suggestion-finished", (event) => {
        const { result, error } = event.payload;
        if (result) {
            setState({ status: "done", stopped: result.stopped, stats: result.stats, suggestions: result.suggestions });
        } else {
            setState({ status: "error", error: error ?? "探せませんでした" });
        }
    });
};

const isHiragana = (reading: string) => /^[ぁ-ゖー]+$/.test(reading);

const LLM_HOST = "192.168.1.184:8092";

const formatBytes = (bytes: number) => `${(bytes / 1e9).toFixed(1)} GB`;
const formatNumber = (value: number) => value.toLocaleString("ja-JP");

// 用例の中の語に印を付ける
const Highlighted = ({ text, word }: { text: string; word: string }) => {
    const parts = text.split(word);
    return (
        <>
            {parts.map((part, index) => (
                <span key={index}>
                    {part}
                    {index < parts.length - 1 && <mark className="rounded-sm bg-amber-200 px-0.5 text-foreground dark:bg-amber-800">{word}</mark>}
                </span>
            ))}
        </>
    );
};

const Step = ({ label, detail, status }: { label: string; detail: string; status: "done" | "now" | "wait" }) => (
    <li className={`grid grid-cols-[18px_1fr_auto] items-center gap-2 ${status === "wait" ? "text-muted-foreground" : ""}`}>
        <span
            aria-hidden="true"
            className={`size-2.5 justify-self-center rounded-full border-2 ${
                status === "done"
                    ? "border-green-600 bg-green-600"
                    : status === "now"
                      ? "animate-spin border-foreground border-t-transparent motion-reduce:animate-none"
                      : "border-border"
            }`}
        />
        <span>
            {label}
            <span className="sr-only">（{status === "done" ? "済み" : status === "now" ? "実行中" : "未着手"}）</span>
        </span>
        <span className="text-xs text-muted-foreground tabular-nums">{detail}</span>
    </li>
);

const RunningView = ({ progress, stopping }: { progress: Progress | null; stopping: boolean }) => {
    const phase = progress?.phase ?? "connect";
    if (phase === "connect") {
        return (
            <ul className="space-y-2 text-sm">
                <Step label="4090 機の LLM に接続を確認しています…" detail="" status="now" />
            </ul>
        );
    }
    const order: Phase[] = ["read", "check", "reading"];
    const current = order.indexOf(phase);
    const statusOf = (step: Phase) => {
        const index = order.indexOf(step);
        return index < current ? "done" : index === current ? "now" : "wait";
    };
    const detailOf = (step: Phase) => {
        if (statusOf(step) === "wait" || !progress) return "";
        if (step === "read") {
            return statusOf(step) === "done" ? "済み" : `${formatBytes(progress.done)} / ${formatBytes(progress.total)}`;
        }
        if (statusOf(step) === "done") return "済み";
        return `${formatNumber(progress.done)} / ${formatNumber(progress.total)} 語`;
    };
    const percent = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;
    const labels: Record<Phase, string> = {
        connect: "",
        read: "材料を読む",
        check: "変換できる語を落とす",
        reading: "読みを付ける",
    };
    return (
        <div className="space-y-3">
            <ul className="space-y-2 text-sm">
                {order.map((step) => (
                    <Step key={step} label={labels[step]} detail={detailOf(step)} status={statusOf(step)} />
                ))}
            </ul>
            <div
                className="h-1.5 overflow-hidden rounded-full bg-muted"
                role="progressbar"
                aria-label={labels[phase]}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={percent}
            >
                <div className="h-full bg-foreground transition-[width]" style={{ width: `${percent}%` }} />
            </div>
            {stopping && <p className="text-xs text-muted-foreground">止めています…</p>}
        </div>
    );
};

const SuggestionRow = ({
    suggestion,
    selected,
    expanded,
    onToggleSelect,
    onToggleExpand,
    onChangeReading,
}: {
    suggestion: Suggestion;
    selected: boolean;
    expanded: boolean;
    onToggleSelect: () => void;
    onToggleExpand: () => void;
    onChangeReading: (reading: string) => void;
}) => {
    const invalid = !isHiragana(suggestion.reading);
    const id = `suggestion-${encodeURIComponent(suggestion.word)}`;
    return (
        <li className="border-b last:border-b-0">
            <div className="grid grid-cols-[16px_minmax(0,1fr)_9.5rem_2rem] items-center gap-2 px-2.5 py-2">
                <input
                    type="checkbox"
                    className="size-4 accent-foreground"
                    checked={selected}
                    onChange={onToggleSelect}
                    aria-label={`「${suggestion.word}」を選ぶ`}
                />
                <div className="min-w-0">
                    <div className="text-sm font-semibold break-all">
                        {suggestion.word}
                        <span className="ml-1.5 text-xs font-normal text-muted-foreground tabular-nums">{formatNumber(suggestion.count)} 回</span>
                    </div>
                    {suggestion.examples[0] && (
                        <div className="truncate text-xs text-muted-foreground" title={suggestion.examples[0].text}>
                            {suggestion.examples[0].text}
                        </div>
                    )}
                </div>
                <div>
                    <Input
                        className={`h-8 ${invalid ? "border-destructive" : ""}`}
                        value={suggestion.reading}
                        onChange={(event) => onChangeReading(event.target.value)}
                        aria-label={`「${suggestion.word}」の読み`}
                        aria-invalid={invalid}
                        aria-describedby={invalid ? `${id}-error` : undefined}
                    />
                    {invalid && (
                        <div id={`${id}-error`} className="mt-0.5 text-[11px] text-destructive">
                            ひらがなで入力してください
                        </div>
                    )}
                </div>
                <Button
                    size="icon"
                    variant="ghost"
                    className="size-8"
                    onClick={onToggleExpand}
                    aria-expanded={expanded}
                    aria-controls={`${id}-examples`}
                    aria-label={`「${suggestion.word}」の用例を${expanded ? "閉じる" : "開く"}`}
                >
                    <ChevronDown className={`transition-transform motion-reduce:transition-none ${expanded ? "rotate-180" : ""}`} />
                </Button>
            </div>
            {expanded && (
                <ul id={`${id}-examples`} className="space-y-1.5 pr-2.5 pb-2.5 pl-[34px]">
                    {suggestion.examples.map((example, index) => (
                        <li key={index} className="rounded-md bg-muted px-2 py-1.5 text-xs leading-relaxed break-all">
                            <span className="block text-[11px] text-muted-foreground">
                                {example.source === "conversation" ? "会話" : "ノート"}
                            </span>
                            <Highlighted text={example.text} word={suggestion.word} />
                        </li>
                    ))}
                </ul>
            )}
        </li>
    );
};

const ResultView = ({
    stopped,
    stats,
    suggestions,
    registered,
    busy,
    onRegister,
}: {
    stopped: boolean;
    stats: ScanStats;
    suggestions: Suggestion[];
    registered: Entry[];
    busy: boolean;
    onRegister: (entries: Entry[]) => Promise<boolean>;
}) => {
    const [selected, setSelected] = useState<Set<string>>(new Set());
    const [expanded, setExpanded] = useState<Set<string>>(new Set());
    const [confirming, setConfirming] = useState(false);

    const update = (next: Suggestion[]) => {
        if (state.status === "done") setState({ ...state, suggestions: next });
    };
    const toggle = (set: Set<string>, word: string) => {
        const next = new Set(set);
        if (next.has(word)) next.delete(word);
        else next.add(word);
        return next;
    };

    const chosen = suggestions.filter((suggestion) => selected.has(suggestion.word));
    const invalidCount = chosen.filter((suggestion) => !isHiragana(suggestion.reading)).length;
    const allSelected = suggestions.length > 0 && chosen.length === suggestions.length;

    const removeFromList = (words: string[]) => {
        update(suggestions.filter((suggestion) => !words.includes(suggestion.word)));
        setSelected(new Set());
    };

    const handleRegister = async () => {
        setConfirming(false);
        // すでに同じ読みと単語の組があれば足さない
        const entries = chosen
            .map((suggestion) => ({ reading: suggestion.reading, word: suggestion.word }))
            .filter((entry) => !registered.some((item) => item.reading === entry.reading && item.word === entry.word));
        if (await onRegister(entries)) {
            removeFromList(chosen.map((suggestion) => suggestion.word));
            toast(`${chosen.length} 語を登録しました`);
        }
    };

    const handleReject = async () => {
        const words = chosen.map((suggestion) => suggestion.word);
        try {
            await invoke("reject_suggestions", { words });
            removeFromList(words);
            toast(`${words.length} 語を却下しました。次からは出ません`);
        } catch {
            toast("却下した語を保存できませんでした");
        }
    };

    const summary = stopped
        ? `途中で止めました。ここまでに見つかった ${suggestions.length} 語です。`
        : `${suggestions.length} 語・${Math.round(stats.seconds)} 秒（材料 ${formatNumber(stats.texts)} 件・調べた語 ${formatNumber(stats.checked)}・読みを付けた語 ${formatNumber(stats.llm)}・${stats.model}）。`;

    if (suggestions.length === 0) {
        return (
            <p className="text-sm leading-relaxed">
                {stopped
                    ? "途中で止めました。ここまでに候補は見つかりませんでした。"
                    : `新しい提案はありません。${formatNumber(stats.checked)} 語を調べ、どれも登録しなくても変換できるか、登録済み・却下済みでした。`}
            </p>
        );
    }

    return (
        <div className="space-y-2">
            <p className="text-xs leading-relaxed text-muted-foreground">
                {summary}チェックした語だけに「登録」「却下」が効きます。却下した語は次から出ません。
            </p>
            <div className="max-h-[60vh] overflow-y-auto rounded-md border">
                <div className="sticky top-0 z-10 flex items-center gap-2 border-b bg-background px-2.5 py-2 text-sm">
                    <input
                        id="suggestion-select-all"
                        type="checkbox"
                        className="size-4 accent-foreground"
                        checked={allSelected}
                        onChange={() => setSelected(allSelected ? new Set() : new Set(suggestions.map((s) => s.word)))}
                    />
                    <label htmlFor="suggestion-select-all" className="flex-1">
                        {chosen.length} / {suggestions.length} 語を選択中
                        {invalidCount > 0 && <span className="text-destructive">（読みを直す語 {invalidCount}）</span>}
                    </label>
                    <Button size="sm" variant="outline" className="text-destructive" disabled={busy || chosen.length === 0} onClick={handleReject}>
                        却下
                    </Button>
                    <Button size="sm" disabled={busy || chosen.length === 0 || invalidCount > 0} onClick={() => setConfirming(true)}>
                        登録
                    </Button>
                </div>
                <ul>
                    {suggestions.map((suggestion) => (
                        <SuggestionRow
                            key={suggestion.word}
                            suggestion={suggestion}
                            selected={selected.has(suggestion.word)}
                            expanded={expanded.has(suggestion.word)}
                            onToggleSelect={() => setSelected(toggle(selected, suggestion.word))}
                            onToggleExpand={() => setExpanded(toggle(expanded, suggestion.word))}
                            onChangeReading={(reading) =>
                                update(suggestions.map((item) => (item.word === suggestion.word ? { ...item, reading: reading.trim() } : item)))
                            }
                        />
                    ))}
                </ul>
            </div>
            <AlertDialog open={confirming} onOpenChange={setConfirming}>
                <AlertDialogContent>
                    <AlertDialogHeader>
                        <AlertDialogTitle>{chosen.length} 語をユーザー辞書に登録します</AlertDialogTitle>
                        <AlertDialogDescription>
                            ユーザー辞書の語はふだんの変換で上位に出ます。読みが違うと変換が崩れるので、確かめてから登録してください。
                        </AlertDialogDescription>
                    </AlertDialogHeader>
                    <ul className="max-h-40 list-disc overflow-y-auto pl-5 text-sm leading-7">
                        {chosen.map((suggestion) => (
                            <li key={suggestion.word}>
                                {suggestion.word}（{suggestion.reading}）
                            </li>
                        ))}
                    </ul>
                    <AlertDialogFooter>
                        <AlertDialogCancel>キャンセル</AlertDialogCancel>
                        <AlertDialogAction onClick={handleRegister}>登録する</AlertDialogAction>
                    </AlertDialogFooter>
                </AlertDialogContent>
            </AlertDialog>
        </div>
    );
};

export const SuggestionPanel = ({
    registered,
    busy,
    onRegister,
}: {
    registered: Entry[];
    busy: boolean;
    onRegister: (entries: Entry[]) => Promise<boolean>;
}) => {
    const current = useSyncExternalStore(subscribe, () => state);
    listenOnce();

    const start = async () => {
        setState({ status: "running", progress: null, stopping: false });
        try {
            await invoke("start_suggestion_scan");
        } catch (error) {
            setState({ status: "error", error: String(error) });
        }
    };
    const stop = () => {
        if (state.status === "running") setState({ ...state, stopping: true });
        invoke("stop_suggestion_scan").catch(() => toast("止められませんでした"));
    };

    const running = current.status === "running";

    return (
        <section id="suggestion-panel" className="space-y-2">
            <div className="flex items-center justify-between gap-4">
                <h1 className="text-sm font-bold text-foreground">登録の提案</h1>
                {(current.status === "done" || current.status === "error") && (
                    <Button size="sm" variant="outline" onClick={start}>
                        もう一度探す
                    </Button>
                )}
            </div>
            {current.status === "idle" && (
                <div className="space-y-3 rounded-md border p-4">
                    <p className="text-sm leading-relaxed">
                        会話ログとノートから、よく使うのに変換で出にくい語を探します。材料は読むだけで書き換えません。登録するかどうかは結果を見て決めます。
                    </p>
                    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                        <dt className="text-muted-foreground">会話ログ</dt>
                        <dd className="break-all">%USERPROFILE%\.claude\projects（あなたの発言だけ）</dd>
                        <dt className="text-muted-foreground">ノート</dt>
                        <dd className="break-all">E:\Obsidian-Vault の Knowledge・Projects・Daily</dd>
                        <dt className="text-muted-foreground">読み付け</dt>
                        <dd className="break-all">4090 機の LLM（{LLM_HOST}・いま動いているモデル）</dd>
                    </dl>
                    <Button id="suggestion-start" onClick={start}>
                        会話から探す
                    </Button>
                </div>
            )}
            {running && (
                <div className="space-y-3 rounded-md border p-4" aria-live="polite">
                    <RunningView progress={current.progress} stopping={current.stopping} />
                    <div className="flex items-center gap-3">
                        <Button id="suggestion-stop" variant="outline" onClick={stop} disabled={current.stopping}>
                            停止
                        </Button>
                        <span className="text-xs text-muted-foreground">止めると、ここまでに見つかった候補を出します</span>
                    </div>
                </div>
            )}
            {current.status === "error" && (
                <div role="alert" className="space-y-1 rounded-md border border-destructive/50 p-4 text-sm leading-relaxed">
                    <p className="flex items-center gap-2 font-bold text-destructive">
                        <TriangleAlert className="size-4 shrink-0" />
                        探せませんでした
                    </p>
                    <p className="break-all">{current.error}</p>
                    <p className="text-xs text-muted-foreground">
                        4090 機に届かないときは、4090 機が起動していて llama-swap が動いているかを確かめてから、もう一度探してください。
                    </p>
                </div>
            )}
            {current.status === "done" && (
                <ResultView
                    stopped={current.stopped}
                    stats={current.stats}
                    suggestions={current.suggestions}
                    registered={registered}
                    busy={busy}
                    onRegister={onRegister}
                />
            )}
        </section>
    );
};
