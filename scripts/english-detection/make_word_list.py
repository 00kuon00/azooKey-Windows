"""同梱する英単語の一覧（server-swift/EnglishWords/english_words.txt）を SCOWL（ESDB）から作る。

SCOWL は github.com/en-wl/wordlist の 1e5b7d3（2026-06-24）。手順:
    git fetch --depth 1 https://github.com/en-wl/wordlist 1e5b7d3a72f47a71da5d28686c1dd4b397178485
    PYTHONUTF8=1 python combine.py create-db scowl.db
    for n in 35 50 60 70 80: PYTHONUTF8=1 python scowl --db scowl.db word-list $n A 1 --deaccent > wl-$n.txt
    python -I make_word_list.py --lists <wl-*.txt のフォルダ> --max-size <大きさ> --out server-swift/EnglishWords/english_words.txt

アメリカ英語（A）・大きさ 80 以下だけを使う（80 を超える一覧とオーストラリア英語は Copyright の追加の条項が付くため）。
著作権表示と許諾文は SCOWL の Copyright をそのまま server-swift/EnglishWords/SCOWL-Copyright.txt に置く。

出力は 1 行 1 語の「語<TAB>大きさ」。語は小文字にまとめ、英字だけの 2 文字以上の語を残す。
大きさは、その語が初めて現れる一覧の大きさ（小さいほどよく使う語）。
"""

import argparse
import os
import re

SIZES = (35, 50, 60, 70, 80)
WORD = re.compile(r"[A-Za-z]{2,}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lists", required=True)
    parser.add_argument("--max-size", type=int, default=80, choices=SIZES)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    sizes = {}
    for size in SIZES:
        if size > args.max_size:
            break
        with open(os.path.join(args.lists, f"wl-{size}.txt"), encoding="utf-8") as f:
            for line in f:
                word = line.strip()
                if not WORD.fullmatch(word):
                    continue
                sizes.setdefault(word.lower(), size)
    with open(args.out, "w", encoding="utf-8", newline="\n") as f:
        for word in sorted(sizes):
            f.write(f"{word}\t{sizes[word]}\n")
    print(len(sizes))


if __name__ == "__main__":
    main()
