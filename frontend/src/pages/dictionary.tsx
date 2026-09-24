import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { BookA, Check, Pencil, Plus, Search, Trash2, TriangleAlert, X } from "lucide-react";
import { FormEvent, ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner"
import { invoke } from '@tauri-apps/api/core';

// %APPDATA%\Azookey\user_dictionary.json の要素と同じ形
type Entry = { reading: string; word: string };

type UserDictionary = {
    entries: Entry[];
    // 読みに辞書で使えない文字があって、変換に使えなかった語
    unregistered: Entry[];
};

// 一覧の 1 行の高さ（px）。見えている行だけを描くので、全行を同じ高さにする
const ROW_HEIGHT = 44;
// 見えている範囲の上下に余分に描く行数（速くスクロールしたときに空白が見えないように）
const OVERSCAN = 8;

const sameEntry = (a: Entry, b: Entry) => a.reading === b.reading && a.word === b.word;

const normalize = (entry: Entry): Entry => ({ reading: entry.reading.trim(), word: entry.word.trim() });

// 数千件でも重くならないよう、スクロール位置から見えている行だけを描く
const VirtualList = <T,>({
    items,
    renderRow,
    getKey,
}: {
    items: T[];
    renderRow: (item: T) => ReactNode;
    getKey: (item: T) => string;
}) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const [scrollTop, setScrollTop] = useState(0);
    const [viewportHeight, setViewportHeight] = useState(0);

    useEffect(() => {
        const container = containerRef.current;
        if (!container) return;
        const observer = new ResizeObserver(() => setViewportHeight(container.clientHeight));
        observer.observe(container);
        return () => observer.disconnect();
    }, []);

    // 絞り込みで件数が減ったとき、範囲外のスクロール位置に残らないようにする
    useEffect(() => {
        const container = containerRef.current;
        if (container && container.scrollTop > items.length * ROW_HEIGHT) {
            container.scrollTop = 0;
        }
    }, [items.length]);

    const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
    const end = Math.min(items.length, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN);

    return (
        <div
            ref={containerRef}
            className="max-h-[60vh] overflow-y-auto"
            onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        >
            <ul className="relative" style={{ height: items.length * ROW_HEIGHT }}>
                {items.slice(start, end).map((item, offset) => (
                    <li
                        key={getKey(item)}
                        className="absolute inset-x-0 border-b"
                        style={{ top: (start + offset) * ROW_HEIGHT, height: ROW_HEIGHT }}
                    >
                        {renderRow(item)}
                    </li>
                ))}
            </ul>
        </div>
    );
};

const EntryRow = ({
    entry,
    isUnregistered,
    isEditing,
    busy,
    onStartEdit,
    onCancelEdit,
    onSave,
    onDelete,
}: {
    entry: Entry;
    isUnregistered: boolean;
    isEditing: boolean;
    busy: boolean;
    onStartEdit: () => void;
    onCancelEdit: () => void;
    onSave: (next: Entry) => void;
    onDelete: () => void;
}) => {
    const [draft, setDraft] = useState(entry);

    useEffect(() => {
        if (isEditing) setDraft(entry);
    }, [isEditing, entry]);

    if (isEditing) {
        const submit = (event: FormEvent) => {
            event.preventDefault();
            onSave(draft);
        };
        return (
            <form
                className="flex h-full items-center gap-2 px-2"
                onSubmit={submit}
                onKeyDown={(event) => event.key === "Escape" && onCancelEdit()}
            >
                <Input
                    aria-label="読み"
                    className="h-8 flex-1"
                    value={draft.reading}
                    onChange={(event) => setDraft({ ...draft, reading: event.target.value })}
                    autoFocus
                />
                <Input
                    aria-label="単語"
                    className="h-8 flex-1"
                    value={draft.word}
                    onChange={(event) => setDraft({ ...draft, word: event.target.value })}
                />
                <Button type="submit" size="icon" variant="ghost" className="size-8" aria-label="保存" disabled={busy}>
                    <Check />
                </Button>
                <Button type="button" size="icon" variant="ghost" className="size-8" aria-label="キャンセル" onClick={onCancelEdit}>
                    <X />
                </Button>
            </form>
        );
    }

    return (
        <div className="flex h-full items-center gap-2 px-2 text-sm">
            <span className="flex flex-1 items-center gap-1.5 truncate" title={entry.reading}>
                {isUnregistered && (
                    <TriangleAlert className="size-4 shrink-0 text-destructive" aria-label="登録できなかった語" />
                )}
                <span className="truncate">{entry.reading}</span>
            </span>
            <span className="flex-1 truncate" title={entry.word}>{entry.word}</span>
            <Button size="icon" variant="ghost" className="size-8" aria-label={`「${entry.word}」を編集`} onClick={onStartEdit} disabled={busy}>
                <Pencil />
            </Button>
            <Button size="icon" variant="ghost" className="size-8" aria-label={`「${entry.word}」を削除`} onClick={onDelete} disabled={busy}>
                <Trash2 />
            </Button>
        </div>
    );
};

export const Dictionary = () => {
    const [entries, setEntries] = useState<Entry[]>([]);
    const [unregistered, setUnregistered] = useState<Entry[]>([]);
    const [loaded, setLoaded] = useState(false);
    const [loadError, setLoadError] = useState(false);
    const [busy, setBusy] = useState(false);
    const [filter, setFilter] = useState("");
    const [draft, setDraft] = useState<Entry>({ reading: "", word: "" });
    // 編集中の行（読みと単語の組で特定する。同じ組は登録させない）
    const [editing, setEditing] = useState<Entry | null>(null);

    useEffect(() => {
        invoke<UserDictionary>("get_user_dictionary")
            .then((data) => {
                setEntries(data.entries);
                setUnregistered(data.unregistered);
                setLoaded(true);
            })
            .catch(() => setLoadError(true));
    }, []);

    // 保存して辞書を作り直させる。失敗したら一覧は元のまま
    const save = async (next: Entry[]) => {
        setBusy(true);
        try {
            const result = await invoke<Entry[]>("save_user_dictionary", { entries: next });
            setEntries(next);
            setUnregistered(result);
            return true;
        } catch (error) {
            toast("辞書の保存に失敗しました");
            return false;
        } finally {
            setBusy(false);
        }
    };

    const validate = (entry: Entry, except?: Entry) => {
        if (!entry.reading || !entry.word) {
            toast("読みと単語を入力してください");
            return false;
        }
        if (entries.some((item) => sameEntry(item, entry) && !(except && sameEntry(item, except)))) {
            toast("同じ読みと単語の組がすでに登録されています");
            return false;
        }
        return true;
    };

    const handleAdd = async (event: FormEvent) => {
        event.preventDefault();
        const entry = normalize(draft);
        if (!validate(entry)) return;
        if (await save([...entries, entry])) {
            setDraft({ reading: "", word: "" });
            // 続けて登録できるよう、読みの欄へ戻す（Input は ref を受け取らないので id で引く）
            document.getElementById("dictionary-new-reading")?.focus();
        }
    };

    const handleEdit = async (original: Entry, next: Entry) => {
        const entry = normalize(next);
        if (!validate(entry, original)) return;
        if (await save(entries.map((item) => (sameEntry(item, original) ? entry : item)))) {
            setEditing(null);
        }
    };

    const handleDelete = async (target: Entry) => {
        if (await save(entries.filter((item) => !sameEntry(item, target)))) {
            toast(`「${target.word}」を削除しました`);
        }
    };

    // 新しく追加した語が上に来るよう、逆順で表示する
    const visible = useMemo(() => {
        const query = filter.trim();
        const reversed = [...entries].reverse();
        return query ? reversed.filter((entry) => entry.reading.includes(query)) : reversed;
    }, [entries, filter]);

    return (
        <div className="space-y-8">
            <section className="space-y-2">
                <h1 className="text-sm font-bold text-foreground">単語を追加</h1>
                <form className="flex items-end gap-2 rounded-md border p-4" onSubmit={handleAdd}>
                    <BookA className="mb-2 shrink-0" />
                    <label className="flex-1 space-y-1">
                        <span className="text-xs text-muted-foreground">読み（ひらがな）</span>
                        <Input
                            id="dictionary-new-reading"
                            value={draft.reading}
                            placeholder="あずきー"
                            onChange={(event) => setDraft({ ...draft, reading: event.target.value })}
                        />
                    </label>
                    <label className="flex-1 space-y-1">
                        <span className="text-xs text-muted-foreground">単語</span>
                        <Input
                            value={draft.word}
                            placeholder="azooKey"
                            onChange={(event) => setDraft({ ...draft, word: event.target.value })}
                        />
                    </label>
                    <Button type="submit" disabled={busy || !loaded}>
                        <Plus />
                        追加
                    </Button>
                </form>
            </section>

            {unregistered.length > 0 && (
                <section className="space-y-2" aria-live="polite">
                    <h1 className="text-sm font-bold text-foreground">登録できなかった語（{unregistered.length} 件）</h1>
                    <div className="space-y-2 rounded-md border border-destructive/50 p-4">
                        <p className="flex items-center gap-2 text-xs text-muted-foreground">
                            <TriangleAlert className="size-4 shrink-0 text-destructive" />
                            読みに変換で使えない文字（漢字や一部の記号など）が入っているため、変換に出ません。読みを直してください。
                        </p>
                        <ul className="max-h-40 space-y-1 overflow-y-auto text-sm">
                            {unregistered.map((entry) => (
                                <li key={`${entry.reading}\t${entry.word}`} className="flex gap-4">
                                    <span className="flex-1 truncate">{entry.reading || "（読みなし）"}</span>
                                    <span className="flex-1 truncate">{entry.word}</span>
                                </li>
                            ))}
                        </ul>
                    </div>
                </section>
            )}

            <section className="space-y-2">
                <div className="flex items-center justify-between gap-4">
                    <h1 className="text-sm font-bold text-foreground">
                        登録した単語（{filter.trim() ? `${visible.length} / ${entries.length}` : entries.length} 件）
                    </h1>
                    <label className="relative w-56">
                        <span className="sr-only">読みで絞り込む</span>
                        <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                        <Input
                            className="h-8 pl-8"
                            placeholder="読みで絞り込む"
                            value={filter}
                            onChange={(event) => setFilter(event.target.value)}
                        />
                    </label>
                </div>
                <div className="rounded-md border">
                    <div className="flex gap-2 border-b px-2 py-2 text-xs text-muted-foreground" aria-hidden="true">
                        <span className="flex-1">読み</span>
                        <span className="flex-1">単語</span>
                        <span className="w-[72px]" />
                    </div>
                    {loadError ? (
                        <p className="p-4 text-sm text-muted-foreground">辞書を読み込めませんでした。変換エンジンが起動しているか確かめてください。</p>
                    ) : !loaded ? (
                        <p className="p-4 text-sm text-muted-foreground">読み込み中…</p>
                    ) : visible.length === 0 ? (
                        <p className="p-4 text-sm text-muted-foreground">
                            {entries.length === 0 ? "まだ単語が登録されていません" : "一致する単語がありません"}
                        </p>
                    ) : (
                        <VirtualList
                            items={visible}
                            getKey={(entry) => `${entry.reading}\t${entry.word}`}
                            renderRow={(entry) => (
                                <EntryRow
                                    entry={entry}
                                    isUnregistered={unregistered.some((item) => sameEntry(item, entry))}
                                    isEditing={editing !== null && sameEntry(editing, entry)}
                                    busy={busy}
                                    onStartEdit={() => setEditing(entry)}
                                    onCancelEdit={() => setEditing(null)}
                                    onSave={(next) => handleEdit(entry, next)}
                                    onDelete={() => handleDelete(entry)}
                                />
                            )}
                        />
                    )}
                </div>
            </section>
        </div>
    )
}
