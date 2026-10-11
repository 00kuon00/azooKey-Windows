import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
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
import { TriangleAlert } from "lucide-react";
import { useEffect, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// 日英の自動判定（#26）の設定。日本語入力のまま打った英単語を、英字のまま入れた文として帯の候補に出す。
// 部品の呼び名: #english-enable（ON/OFF）・#english-vocabulary（自分の語彙）・#english-rebuild（作り直す）・
// #english-words（英字の候補に入れる語・入れない語の一覧）

type Source = "own" | "learned" | "excluded";
type WordRow = { word: string; source: Source; count: number };
type VocabularySummary = { words: number; removed: number; builtAt: string };
type WordList = { vocabulary: VocabularySummary | null; english: WordRow[]; excluded: WordRow[] };
type Phase = "read" | "readings" | "words";
type Progress = { phase: Phase; done: number; total: number };
type Finished = { list: WordList | null; stopped: boolean; error: string | null };

type BuildState =
    | { status: "idle" }
    | { status: "running"; progress: Progress | null; stopping: boolean }
    | { status: "error"; error: string }
    | { status: "done"; stopped: boolean };

// 作り直している途中で別のページへ移っても、戻ったときに進み具合が残るよう、状態はページの外に置く
let buildState: BuildState = { status: "idle" };
let latestList: WordList | null = null;
const subscribers = new Set<() => void>();
const notify = () => subscribers.forEach((callback) => callback());
const setBuildState = (next: BuildState) => {
    buildState = next;
    notify();
};
const setLatestList = (next: WordList) => {
    latestList = next;
    notify();
};
const subscribe = (callback: () => void) => {
    subscribers.add(callback);
    return () => subscribers.delete(callback);
};

let listening = false;
const listenOnce = () => {
    if (listening) return;
    listening = true;
    listen<Progress>("english-vocabulary-progress", (event) => {
        if (buildState.status === "running") setBuildState({ ...buildState, progress: event.payload });
    });
    listen<Finished>("english-vocabulary-finished", (event) => {
        const { list, stopped, error } = event.payload;
        if (list) setLatestList(list);
        if (error) setBuildState({ status: "error", error });
        else setBuildState({ status: "done", stopped });
    });
};

const formatNumber = (value: number) => value.toLocaleString("ja-JP");
const formatDate = (iso: string) => {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : date.toLocaleString("ja-JP", { dateStyle: "medium", timeStyle: "short" });
};
const sourceLabel: Record<Source, string> = { own: "自分の語彙", learned: "覚えた語", excluded: "英字にしない語" };
// 一覧は多い（数千語）ので、探していないときは上から少しだけ出す
const PAGE = 200;

const progressText = (progress: Progress | null) => {
    if (!progress) return "準備しています…";
    const percent = progress.total > 0 ? Math.floor((progress.done / progress.total) * 100) : 0;
    switch (progress.phase) {
        case "read":
            return `会話ログとノートを読んでいます… ${percent}%`;
        case "readings":
            return `日本語の部分に読みを付けています… ${formatNumber(progress.done)} / ${formatNumber(progress.total)}`;
        case "words":
            return `語を確かめています…`;
    }
};

const EnableSwitch = () => {
    const [enabled, setEnabled] = useState(true);
    useEffect(() => {
        invoke<any>("get_config")
            .then((data) => setEnabled(data.english?.enable ?? true))
            .catch(() => {});
    }, []);
    const change = async (value: boolean) => {
        try {
            const data = await invoke<any>("get_config");
            data.english = { ...data.english, enable: value };
            await invoke("update_config", { newConfig: data });
            setEnabled(value);
        } catch {
            toast("設定の更新に失敗しました");
        }
    };
    return (
        <div className="flex items-center justify-between gap-6 rounded-md border p-4">
            <label htmlFor="english-enable" className="space-y-1">
                <p className="text-sm font-bold">英単語を帯に英字の候補として出す</p>
                <p className="text-xs text-muted-foreground leading-relaxed">
                    日本語入力のまま打った英単語（google・claude）を、英字のまま入れた文として候補ウィンドウの帯に出します。Tab で選んで Enter で確定します。ライブ変換は変わりません。
                </p>
            </label>
            <Switch id="english-enable" checked={enabled} onCheckedChange={change} />
        </div>
    );
};

const VocabularyPanel = ({ list }: { list: WordList | null }) => {
    const current = useSyncExternalStore(subscribe, () => buildState);
    const start = async () => {
        setBuildState({ status: "running", progress: null, stopping: false });
        try {
            await invoke("start_english_vocabulary_build");
        } catch (error) {
            setBuildState({ status: "error", error: String(error) });
        }
    };
    const stop = () => {
        if (buildState.status === "running") setBuildState({ ...buildState, stopping: true });
        invoke("stop_english_vocabulary_build").catch(() => toast("止められませんでした"));
    };
    const vocabulary = list?.vocabulary ?? null;

    if (current.status === "running") {
        const { progress, stopping } = current;
        const percent = progress && progress.total > 0 ? (progress.done / progress.total) * 100 : 0;
        return (
            <div className="space-y-3 rounded-md border p-4" aria-live="polite">
                <p className="text-sm">{stopping ? "止めています…" : progressText(progress)}</p>
                <div className="h-1.5 overflow-hidden rounded-full bg-muted" role="progressbar" aria-valuenow={Math.round(percent)} aria-valuemin={0} aria-valuemax={100}>
                    <div className="h-full bg-primary transition-[width] motion-reduce:transition-none" style={{ width: `${percent}%` }} />
                </div>
                <p className="text-xs text-muted-foreground">作り直している間も入力は止まりません。止めると、今までの語彙のままにします。</p>
                <Button id="english-stop" variant="outline" size="sm" onClick={stop} disabled={stopping}>
                    停止
                </Button>
            </div>
        );
    }
    return (
        <div className="space-y-3">
            {current.status === "error" && (
                <div role="alert" className="space-y-1 rounded-md border border-destructive/50 p-4 text-sm leading-relaxed">
                    <p className="flex items-center gap-2 font-bold text-destructive">
                        <TriangleAlert className="size-4 shrink-0" />
                        作り直せませんでした
                    </p>
                    <p className="break-all">{current.error}</p>
                    <p className="text-xs text-muted-foreground">今までの語彙はそのまま使っています。</p>
                </div>
            )}
            {current.status === "done" && current.stopped && (
                <p className="text-xs text-muted-foreground" role="status">止めました。今までの語彙のままです。</p>
            )}
            {vocabulary ? (
                <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-4">
                    <p className="text-sm">
                        {formatNumber(vocabulary.words)} 語 ・ {formatDate(vocabulary.builtAt)} に作成（日本語と重なる {formatNumber(vocabulary.removed)} 語を外した）
                    </p>
                    <Button id="english-rebuild" variant="outline" size="sm" onClick={start}>
                        作り直す
                    </Button>
                </div>
            ) : (
                <div className="space-y-3 rounded-md border p-4">
                    <p className="text-sm leading-relaxed">
                        会話ログとノートから、よく使う英字の語（3 回以上）を集めます。日本語として打つローマ字（made・demo）と重なる語は外します。材料は読むだけで書き換えません。作るまでは、同梱の英単語の一覧だけで判定します。
                    </p>
                    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                        <dt className="text-muted-foreground">会話ログ</dt>
                        <dd className="break-all">%USERPROFILE%\.claude\projects（あなたの発言だけ）</dd>
                        <dt className="text-muted-foreground">ノート</dt>
                        <dd className="break-all">E:\Obsidian-Vault の Knowledge・Projects・Daily</dd>
                    </dl>
                    <Button id="english-rebuild" onClick={start}>
                        自分の語彙を作る
                    </Button>
                </div>
            )}
        </div>
    );
};

const WordsPanel = ({ list }: { list: WordList | null }) => {
    const [mode, setMode] = useState<"english" | "excluded">("english");
    const [query, setQuery] = useState("");
    const [confirming, setConfirming] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const rows = (mode === "english" ? list?.english : list?.excluded) ?? [];
    const filtered = query ? rows.filter((row) => row.word.includes(query.trim().toLowerCase())) : rows;
    const shown = filtered.slice(0, PAGE);

    const setDirection = async (word: string, direction: "japanese" | null) => {
        setBusy(true);
        try {
            setLatestList(await invoke<WordList>("set_english_word", { word, direction }));
            toast(direction ? `${word} を外しました` : `${word} を戻しました`);
        } catch (error) {
            toast(`変更できませんでした: ${error}`);
        } finally {
            setBusy(false);
        }
    };

    return (
        <section id="english-words" className="space-y-3 rounded-md border p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex gap-2" role="group" aria-label="一覧の種類">
                    <Button size="sm" variant={mode === "english" ? "default" : "outline"} aria-pressed={mode === "english"} onClick={() => setMode("english")}>
                        英字にする語 {formatNumber(list?.english.length ?? 0)}
                    </Button>
                    <Button size="sm" variant={mode === "excluded" ? "default" : "outline"} aria-pressed={mode === "excluded"} onClick={() => setMode("excluded")}>
                        英字にしない語 {formatNumber(list?.excluded.length ?? 0)}
                    </Button>
                </div>
                <Input id="english-word-search" type="search" placeholder="語を探す" aria-label="語を探す" className="h-8 max-w-60" value={query} onChange={(e) => setQuery(e.target.value)} />
            </div>
            {shown.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                    {query
                        ? `「${query}」に当たる語はありません`
                        : mode === "english"
                          ? "まだありません。自分の語彙を作るか、F10 で英字に直して確定すると、ここに並びます。"
                          : "まだありません。F6 でひらがなに直して確定した語・外した語がここに並びます。"}
                </p>
            ) : (
                <div className="overflow-x-auto">
                    <table className="w-full text-sm">
                        <thead>
                            <tr className="text-xs text-muted-foreground">
                                <th className="py-1 pr-3 text-left font-medium">語</th>
                                <th className="py-1 pr-3 text-left font-medium">出どころ</th>
                                <th className="py-1 pr-3 text-right font-medium">回数</th>
                                <th className="py-1"></th>
                            </tr>
                        </thead>
                        <tbody>
                            {shown.map((row) => (
                                <tr key={row.word} className="border-t">
                                    <td className="py-1.5 pr-3">{row.word}</td>
                                    <td className="py-1.5 pr-3">
                                        <span className={"rounded-full border px-2 text-xs " + (row.source === "learned" ? "border-primary text-primary" : "text-muted-foreground")}>
                                            {sourceLabel[row.source]}
                                        </span>
                                    </td>
                                    <td className="py-1.5 pr-3 text-right tabular-nums">{row.count > 0 ? formatNumber(row.count) : "—"}</td>
                                    <td className="py-1.5 text-right">
                                        {mode === "english" ? (
                                            <Button size="sm" variant="outline" className="text-destructive" disabled={busy} onClick={() => setConfirming(row.word)}>
                                                外す
                                            </Button>
                                        ) : (
                                            <Button size="sm" variant="outline" disabled={busy} onClick={() => setDirection(row.word, null)}>
                                                戻す
                                            </Button>
                                        )}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                    {filtered.length > shown.length && (
                        <p className="pt-2 text-xs text-muted-foreground">
                            {formatNumber(filtered.length)} 語のうち {formatNumber(shown.length)} 語を出しています。語を探すと絞れます。
                        </p>
                    )}
                </div>
            )}
            <p className="text-xs text-muted-foreground leading-relaxed">
                「外す」と、その語は英字の候補に入れません（自分の語彙を作り直しても戻りません）。F10 で英字に直した語・帯で選んだ語は「覚えた語」として先に並びます。
            </p>
            <AlertDialog open={confirming !== null} onOpenChange={(open) => !open && setConfirming(null)}>
                <AlertDialogContent>
                    <AlertDialogHeader>
                        <AlertDialogTitle>{confirming} を外しますか</AlertDialogTitle>
                        <AlertDialogDescription>
                            次から、この語を打っても帯に英字の候補を出しません。元に戻すには「英字にしない語」から戻します。
                        </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                        <AlertDialogCancel>やめる</AlertDialogCancel>
                        <AlertDialogAction
                            onClick={() => {
                                if (confirming) setDirection(confirming, "japanese");
                                setConfirming(null);
                            }}
                        >
                            外す
                        </AlertDialogAction>
                    </AlertDialogFooter>
                </AlertDialogContent>
            </AlertDialog>
        </section>
    );
};

export const English = () => {
    listenOnce();
    const list = useSyncExternalStore(subscribe, () => latestList);
    useEffect(() => {
        invoke<WordList>("get_english_words")
            .then(setLatestList)
            .catch((error) => toast(`語の一覧を読めませんでした: ${error}`));
    }, []);
    return (
        <div className="space-y-6">
            <section className="space-y-2">
                <h1 className="text-sm font-bold text-foreground">英単語の判定</h1>
                <EnableSwitch />
            </section>
            <section id="english-vocabulary" className="space-y-2">
                <h1 className="text-sm font-bold text-foreground">自分の語彙</h1>
                <VocabularyPanel list={list} />
            </section>
            <section className="space-y-2">
                <h1 className="text-sm font-bold text-foreground">英字の候補に入れる語・入れない語</h1>
                <WordsPanel list={list} />
            </section>
        </div>
    );
};
