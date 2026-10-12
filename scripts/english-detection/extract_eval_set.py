"""日英の自動判定（#26）の評価セットと、自分の語彙の数えを作る。

会話ログ（~/.claude/projects/*/*.jsonl）と Vault のノートは読むだけで、書き換えない。
出力は --out のフォルダに置く。⚠ 出力はユーザーの発言そのものなので、リポジトリに入れない。

使い方:
    python -I scripts/english-detection/extract_eval_set.py --out <フォルダ> --cutoff 2026-09-21 [--vault <Vault>]

出力:
- units.jsonl      評価に使う「文」。cutoff 以降に人が打った発言（promptSource が typed / queued）から取る。
                   1 行 = {"ts": 時刻, "text": 文}。文の定義は下の split_units
- ja_units.jsonl   同じ期間の、英字を含まない文（日本語だけの入力を英字にしてしまわないかを見る）
- ja_train.txt     cutoff より前に人が打った発言の日本語の部分（1 行 1 つ）。自分の語彙から、
                   日本語として打つローマ字を外すのに使う
- vocab.json       cutoff より前の会話ログ（ユーザー発言）と Vault のノート（更新が cutoff より前）に出る英字の語の
                   回数（小文字にまとめる）。#17 の「登録の提案」と同じ範囲の発言を数える
- stats.json       取り出した数・外した数
"""

import argparse
import collections
import datetime
import glob
import json
import os
import re
import sys

INJECTED = re.compile(r"(?s)<system-reminder>.*?</system-reminder>|<pasted_content[^>]*>.*?</pasted_content>")
CODE_BLOCK = re.compile(r"(?s)```.*?```")
INLINE_CODE = re.compile(r"`[^`\n]*`")
URL = re.compile(r"https?://\S+")

JAPANESE = "ぁ-ゖァ-ヺー一-龯々〆ヶ"
UNIT = re.compile(f"[{JAPANESE}A-Za-z]+")
LATIN = re.compile(r"[A-Za-z]+")
HAS_JAPANESE = re.compile(f"[{JAPANESE}]")
JAPANESE_RUN = re.compile(f"[{JAPANESE}]+")
# 英字の語と続けて書かれると、1 つの技術的な語（パス・ファイル名・バージョン・識別子）や
# 英文の語の一部（you'll の ll）とみなす文字
JOINS = set("0123456789_./\\-@#$%+:'’")


def log_entries(projects):
    """会話ログのユーザー発言を (時刻, 人が打ったか, 文) で返す。#17（suggestion.rs の user_texts_from_log_line）と同じ範囲"""
    for path in glob.glob(os.path.join(projects, "*", "*.jsonl")):
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if '"type":"user"' not in line or '"type":"tool_result"' in line:
                    continue
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                if value.get("type") != "user" or value.get("isMeta") or value.get("isSidechain"):
                    continue
                content = (value.get("message") or {}).get("content")
                if isinstance(content, str):
                    parts = [content]
                elif isinstance(content, list):
                    parts = [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
                else:
                    parts = []
                typed = value.get("promptSource") in ("typed", "queued")
                for text in parts:
                    text = INJECTED.sub("", text).strip()
                    if not text or text.startswith("<") or text.startswith("Base directory for this skill"):
                        continue
                    yield value.get("timestamp", ""), typed, text


def vault_notes(vault):
    for folder in ("Knowledge", "Projects", "Daily"):
        for root, _, files in os.walk(os.path.join(vault, folder)):
            for name in files:
                if name.endswith(".md"):
                    yield os.path.join(root, name)


def clean(text):
    """人が続けて打った部分だけを残す（コード・URL は打ち方が違うので外す）"""
    text = CODE_BLOCK.sub("\n", text)
    text = INLINE_CODE.sub(" ", text)
    return URL.sub(" ", text)


def is_joined(text, start, end):
    """text[start:end] の英字の語が、前後の文字とつながって技術的な語の一部になっているか"""
    before = text[start - 1] if start > 0 else ""
    after = text[end] if end < len(text) else ""
    return before in JOINS or after in JOINS


def split_units(text):
    """文（評価の単位）を取り出す。

    文 = 日本語（ひらがな・カタカナ・漢字・長音）と英字だけが続く、ひと続きの部分。
    空白・句読点・数字・記号で区切る（IME では空白で変換し、句読点で区切って打つため）。
    返り値は (文, 種類)。種類は mixed（日本語と英字が混じる）・japanese（日本語だけ）・
    joined（英字の語が数字や記号とつながっていて外す）・single（1 文字の英字を含むので外す。ローマ字では
    「a」と「あ」を打ち分けられない）・other
    """
    for match in UNIT.finditer(text):
        unit = match.group(0)
        latin = list(LATIN.finditer(unit))
        if not HAS_JAPANESE.search(unit):
            continue
        if not latin:
            yield unit, "japanese"
            continue
        if any(is_joined(text, match.start() + m.start(), match.start() + m.end()) for m in latin):
            yield unit, "joined"
        elif any(len(m.group(0)) == 1 for m in latin):
            yield unit, "single"
        else:
            yield unit, "mixed"


def count_vocab(text, counter):
    """英字の語を数える。前後の文字とつながった語（パス・ファイル名・識別子の一部）は数えない"""
    for m in LATIN.finditer(text):
        if len(m.group(0)) >= 2 and not is_joined(text, m.start(), m.end()):
            counter[m.group(0).lower()] += 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--cutoff", required=True, help="この日（UTC・YYYY-MM-DD）より前で語彙を作り、この日以降で評価する")
    parser.add_argument("--until", default="9999-12-31", help="評価に使う文の終わり（この日より前・UTC）")
    parser.add_argument("--projects", default=os.path.join(os.path.expanduser("~"), ".claude", "projects"))
    parser.add_argument("--vault", default=r"E:\Obsidian-Vault")
    args = parser.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    os.makedirs(args.out, exist_ok=True)

    cutoff = args.cutoff
    vocab = collections.Counter()
    stats = collections.Counter()
    seen_units = set()
    units = []
    ja_units = []
    ja_train = []
    seen_train = set()
    for ts, typed, text in log_entries(args.projects):
        before = ts[:10] < cutoff
        if before:
            count_vocab(clean(text), vocab)
            stats["vocab_log_texts"] += 1
        if not typed:
            continue
        text = clean(text)
        if before:
            for run in JAPANESE_RUN.findall(text):
                if run not in seen_train:
                    seen_train.add(run)
                    ja_train.append(run)
            continue
        if ts[:10] >= args.until:
            continue
        stats["eval_utterances"] += 1
        for unit, kind in split_units(text):
            stats[f"units_{kind}"] += 1
            if kind not in ("mixed", "japanese"):
                continue
            if unit in seen_units:
                stats[f"units_{kind}_duplicate"] += 1
                continue
            seen_units.add(unit)
            (units if kind == "mixed" else ja_units).append({"ts": ts, "text": unit})

    cutoff_time = datetime.datetime.fromisoformat(cutoff).replace(tzinfo=datetime.timezone.utc).timestamp()
    for path in vault_notes(args.vault):
        if os.path.getmtime(path) >= cutoff_time:
            stats["vocab_notes_skipped_new"] += 1
            continue
        with open(path, encoding="utf-8", errors="replace") as f:
            count_vocab(clean(f.read()), vocab)
        stats["vocab_notes"] += 1

    def write_jsonl(name, rows):
        with open(os.path.join(args.out, name), "w", encoding="utf-8") as f:
            for row in rows:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")

    write_jsonl("units.jsonl", units)
    write_jsonl("ja_units.jsonl", ja_units)
    with open(os.path.join(args.out, "ja_train.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(ja_train) + "\n")
    with open(os.path.join(args.out, "vocab.json"), "w", encoding="utf-8") as f:
        json.dump(dict(vocab.most_common()), f, ensure_ascii=False)
    stats["vocab_words"] = len(vocab)
    stats["vocab_words_3plus"] = sum(1 for c in vocab.values() if c >= 3)
    stats["mixed_units"] = len(units)
    stats["japanese_units"] = len(ja_units)
    stats["ja_train_runs"] = len(ja_train)
    with open(os.path.join(args.out, "stats.json"), "w", encoding="utf-8") as f:
        json.dump(dict(sorted(stats.items())), f, ensure_ascii=False, indent=1)
    print(json.dumps(dict(sorted(stats.items())), ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
