use anyhow::{Context as _, Result};
use tao::{
    event_loop::EventLoop,
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{Window, WindowBuilder},
};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        SetWindowLongW, GWL_EXSTYLE, GWL_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_POPUP,
    },
};
use wry::WebViewBuilder;

use crate::UserEvent;

pub fn create_candidate_window(event_loop: &EventLoop<UserEvent>) -> Result<Window> {
    let window = WindowBuilder::new()
        .with_decorations(false)
        .with_title("CandidateList")
        .with_focused(false)
        .with_visible(false)
        .with_undecorated_shadow(false)
        .with_transparent(true)
        .build(&event_loop)
        .context("Failed to create window")?;

    let hwnd = window.hwnd() as *mut std::ffi::c_void;

    // set extended window style
    // https://docs.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
    // https://docs.microsoft.com/en-us/windows/win32/winmsg/window-styles
    unsafe {
        let exnewstyle = WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0;
        SetWindowLongW(HWND(hwnd), GWL_EXSTYLE, exnewstyle as i32);

        let style = WS_POPUP.0;
        SetWindowLongW(HWND(hwnd), GWL_STYLE, style as i32);
    };

    Ok(window)
}

pub fn create_candidate_webview<'a>() -> Result<WebViewBuilder<'a>> {
    // 部品の呼び名: #reading（読みの行）・#assist（帯）・#prediction-row（予測）・#typo-row（もしかして）・#candidate-list（変換候補の一覧）
    let webview_builder = WebViewBuilder::new()
    .with_transparent(true)
    .with_html(
        r##"
        <html>
            <head>
                <style>
                    :root {
                        --border: #E4E4E4;
                        --sub: #7A7A7A;
                        --ghost: #A5A5A5;
                        --typo: #C2410C;
                        --fix: #15803D;
                        --strip: #F6F8FA;
                        --selected-bg: #D4F0FF;
                        --selected-line: #2CB5FF;
                        --segment: #1C1C1C;
                    }
                    body, html {
                        overscroll-behavior: none;
                    }
                    body {
                        margin: 0;
                        padding: 7px;
                        filter: drop-shadow(3px 3px 3px rgba(0, 0, 0, 0.1));
                        font-family: "Yu Gothic UI", "Segoe UI", sans-serif;
                    }
                    [hidden] {
                        display: none !important;
                    }
                    main {
                        width: 100%;
                        height: 100%;
                        padding: 8px;
                        border: 1px solid var(--border);
                        border-radius: 10px;
                        background-color: #FFFFFF;
                        box-sizing: border-box;
                        display: flex;
                        flex-direction: column;
                    }
                    /* 読みの行。長い読みは先頭を省き、打っている末尾を見せる */
                    #reading {
                        font-size: 0.8rem;
                        color: var(--sub);
                        padding: 2px 8px 6px;
                        white-space: nowrap;
                        overflow: hidden;
                        text-overflow: ellipsis;
                        direction: rtl;
                        text-align: left;
                        border-bottom: 1px solid var(--border);
                        margin-bottom: 4px;
                        user-select: none;
                        flex: none;
                    }
                    #reading > span {
                        direction: ltr;
                        unicode-bidi: isolate;
                    }
                    .typo {
                        color: var(--typo);
                        text-decoration: underline wavy;
                        text-underline-offset: 3px;
                    }
                    .fix {
                        color: var(--fix);
                        font-weight: 600;
                    }
                    .segment {
                        color: var(--segment);
                        font-weight: 600;
                    }
                    .segment + span::before {
                        content: "｜";
                    }
                    .ghost {
                        color: var(--ghost);
                    }
                    /* 帯（予測・もしかして）。Tab で選ぶ */
                    #assist {
                        background: var(--strip);
                        border-radius: 6px;
                        margin: 0 0 4px;
                        user-select: none;
                        flex: none;
                    }
                    .assist-row {
                        display: flex;
                        align-items: center;
                        gap: 0.5rem;
                        padding: 0.35rem 0.5rem;
                        font-size: 0.85rem;
                        white-space: nowrap;
                        border-radius: 5px;
                    }
                    .assist-row .text {
                        overflow: hidden;
                        text-overflow: ellipsis;
                    }
                    .assist-row .count {
                        margin-left: auto;
                        font-size: 0.7rem;
                        color: var(--sub);
                        flex: none;
                        font-variant-numeric: tabular-nums;
                    }
                    .assist-row[data-selected] {
                        background: var(--selected-bg);
                        outline: 1px solid var(--selected-line);
                        outline-offset: -1px;
                    }
                    .kbd {
                        border: 1px solid var(--border);
                        border-radius: 3px;
                        padding: 0 4px;
                        font-size: 0.65rem;
                        color: var(--sub);
                        flex: none;
                    }
                    .sub {
                        font-size: 0.75rem;
                        color: var(--sub);
                    }
                    ol {
                        margin: 0;
                        padding: 0;
                        flex: 1;
                        overflow-y: auto;
                        scroll-snap-type: y proximity;
                        list-style-position: inside;
                        list-style-type: none;
                        counter-reset: number 0;
                        user-select: none;
                        cursor: pointer;

                        &::-webkit-scrollbar {
                            width: 5px;
                        }

                        &::-webkit-scrollbar-thumb {
                            background-color: #BCBCBC;
                            border-radius: 10px;
                        }
                    }
                    li {
                        padding: 0.5rem;
                        font-size: 0.9rem;
                        display: flex;
                        align-items: center;
                        scroll-snap-align: start;

                        &::before {
                            content: counter(number);
                            counter-increment: number 1;
                            color: #636363;
                            font-weight: bold;
                            font-size: 0.75rem;
                            margin: 0 0.75rem 0 2;
                            width: 0.75rem;
                        }

                        &[data-selected] {
                            background-color: var(--selected-bg);
                            border-radius: 3px;
                            margin-right: 5px;
                            outline: 1px solid var(--selected-line);
                            outline-offset: -1px;
                        }
                    }
                    footer {
                        display: flex;
                        justify-content: space-between;
                        align-items: center;
                        padding: 8 10 5 10;
                        border-top: 1px solid var(--border);
                        font-size: 0.8rem;
                        user-select: none;
                    }

                    @media (prefers-color-scheme: dark) {
                        :root {
                            --border: #424242;
                            --sub: #A8A8A8;
                            --ghost: #7A7A7A;
                            --typo: #F08A5D;
                            --fix: #5FD08A;
                            --strip: #262626;
                            --selected-bg: #3949AB;
                            --selected-line: #5C6BC0;
                            --segment: #FFFFFF;
                        }
                        body {
                            color: #FFFFFF;
                        }
                        main {
                            background-color: #1E1E1E;
                        }
                        ol::-webkit-scrollbar-thumb {
                            background-color: #757575;
                        }
                        li {
                            color: #E0E0E0;

                            &::before {
                                color: #BDBDBD;
                            }
                        }
                    }
                </style>
                <script>
                    // 直近の表示内容（Rust の view::CandidateView）と、選んでいる場所（SelectionKind: 0 一覧 / 1 予測 / 2 もしかして）
                    let view = { candidates: [], reading: [], predictions: [], typos: [] };
                    let selection = { kind: 0, index: 0 };
                    let lastHeight = 0;

                    // 印付きの文字の部分（view::Span）を span に並べる
                    function spans(parts) {
                        const fragment = document.createDocumentFragment();
                        for (const part of parts) {
                            const span = document.createElement('span');
                            span.textContent = part.text;
                            if (part.mark) span.className = part.mark;
                            fragment.appendChild(span);
                        }
                        return fragment;
                    }

                    function assistRow(id, count, selected) {
                        const element = document.getElementById(id);
                        element.hidden = count === 0;
                        if (selected) element.setAttribute('data-selected', '');
                        else element.removeAttribute('data-selected');
                        return element;
                    }

                    function render() {
                        const readingText = view.reading.map(p => p.text).join('');
                        const pIndex = selection.kind === 1 ? selection.index : 0;
                        const tIndex = selection.kind === 2 ? selection.index : 0;
                        const prediction = view.predictions[pIndex];

                        // 読みの行（予測が読みで始まるなら、続きを薄く出す）
                        const reading = document.getElementById('reading');
                        reading.hidden = readingText === '';
                        const inner = document.createElement('span');
                        inner.appendChild(spans(view.reading));
                        if (prediction && prediction.ghost) inner.appendChild(spans([{ text: prediction.ghost, mark: 'ghost' }]));
                        reading.replaceChildren(inner);

                        // 帯
                        document.getElementById('assist').hidden = view.predictions.length === 0 && view.typos.length === 0;
                        const predictionRow = assistRow('prediction-row', view.predictions.length, selection.kind === 1);
                        if (prediction) {
                            // 読みで始まる予測は読みの行に続きが出ているので、帯には見出しだけ
                            predictionRow.querySelector('.text').textContent = prediction.ghost ? '予測' : '予測 ' + prediction.text;
                            predictionRow.querySelector('.count').textContent = `${pIndex + 1}/${view.predictions.length}`;
                        }
                        const typoRow = assistRow('typo-row', view.typos.length, selection.kind === 2);
                        const typo = view.typos[tIndex];
                        if (typo) {
                            const text = typoRow.querySelector('.text');
                            text.replaceChildren('もしかして ');
                            // 語が直した読みと同じ（ひらがなのまま）なら読みだけを出す
                            const corrected = typo.reading.map(p => p.text).join('');
                            if (typo.text !== corrected) {
                                text.append(typo.text + '　');
                                const sub = document.createElement('span');
                                sub.className = 'sub';
                                sub.appendChild(spans(typo.reading));
                                text.appendChild(sub);
                            } else {
                                text.appendChild(spans(typo.reading));
                            }
                            typoRow.querySelector('.count').textContent = `${tIndex + 1}/${view.typos.length}`;
                        }

                        // 変換候補の一覧
                        const candidateList = document.getElementById('candidate-list');
                        const existingItems = Array.from(candidateList.children);
                        view.candidates.forEach((candidate, index) => {
                            if (existingItems[index]) {
                                existingItems[index].textContent = candidate;
                            } else {
                                const li = document.createElement('li');
                                li.textContent = candidate;
                                candidateList.appendChild(li);
                            }
                        });
                        while (existingItems.length > view.candidates.length) {
                            candidateList.removeChild(existingItems.pop());
                        }
                        const selected = candidateList.querySelector('[data-selected]');
                        if (selected) selected.removeAttribute('data-selected');
                        if (selection.kind === 0 && candidateList.children[selection.index]) {
                            candidateList.children[selection.index].setAttribute('data-selected', '');
                        }

                        fitHeight();
                    }

                    function updateCandidates(next) {
                        view = next;
                        // 帯の中身が減ったら、選択は一覧へ戻す（正しい選択は続く updateSelection で届く）
                        if (selection.kind === 1 && selection.index >= view.predictions.length) selection = { kind: 0, index: 0 };
                        if (selection.kind === 2 && selection.index >= view.typos.length) selection = { kind: 0, index: 0 };
                        render();
                    }

                    function updateSelection(kind, index) {
                        selection = { kind, index };
                        render();
                        if (kind !== 0) return;

                        const candidateList = document.getElementById('candidate-list');
                        if (!candidateList.children[index]) return;
                        const groupSize = 5;
                        const groupIndex = Math.floor(index / groupSize);
                        const scrollToIndex = groupIndex * groupSize;

                        if (index === scrollToIndex || !isElementInView(candidateList.children[index], candidateList)) {
                            candidateList.children[scrollToIndex].scrollIntoView({ behavior: "instant", block: "start", inline: "start" });
                        }
                    }

                    function isElementInView(element, container) {
                        const containerRect = container.getBoundingClientRect();
                        const elementRect = element.getBoundingClientRect();

                        return (
                            elementRect.top >= containerRect.top &&
                            elementRect.bottom <= containerRect.bottom
                        );
                    }

                    // 一覧 5 行ぶん＋読みの行・帯・footer の高さを Rust へ送る（変わったときだけ）
                    function fitHeight() {
                        const candidateList = document.getElementById('candidate-list');
                        let itemHeight = candidateList.children[0] ? candidateList.children[0].offsetHeight : 0;
                        if (!itemHeight) {
                            const li = document.createElement('li');
                            li.textContent = 'あ';
                            candidateList.appendChild(li);
                            itemHeight = li.offsetHeight;
                            candidateList.removeChild(li);
                        }
                        const outer = element => {
                            if (element.hidden) return 0;
                            const style = window.getComputedStyle(element);
                            return element.offsetHeight + parseFloat(style.marginTop) + parseFloat(style.marginBottom);
                        };
                        const main = document.querySelector('main');
                        const mainStyle = window.getComputedStyle(main);
                        const bodyStyle = window.getComputedStyle(document.body);
                        const height = Math.ceil(
                            itemHeight * 5
                            + outer(document.getElementById('reading'))
                            + outer(document.getElementById('assist'))
                            + outer(document.querySelector('footer'))
                            + parseFloat(mainStyle.paddingTop) + parseFloat(mainStyle.paddingBottom)
                            + parseFloat(mainStyle.borderTopWidth) + parseFloat(mainStyle.borderBottomWidth)
                            + parseFloat(bodyStyle.paddingTop) + parseFloat(bodyStyle.paddingBottom)
                        );
                        if (height === lastHeight) return;
                        lastHeight = height;
                        window.ipc.postMessage(JSON.stringify({
                            type: 'resize',
                            height: height
                        }));
                    }

                    window.addEventListener('DOMContentLoaded', () => {
                        setTimeout(fitHeight, 50); // Small delay to ensure rendering is complete
                    });
                </script>
            </head>
            <body style="margin: 0;">
                <main>
                    <div id="reading" hidden></div>
                    <div id="assist" hidden>
                        <div id="prediction-row" class="assist-row" hidden><span class="kbd">Tab</span><span class="text"></span><span class="count"></span></div>
                        <div id="typo-row" class="assist-row" hidden><span class="kbd">Tab</span><span class="text"></span><span class="count"></span></div>
                    </div>
                    <ol id="candidate-list">
                    </ol>
                    <footer>
                        <svg width="20" height="14" viewBox="0 0 22 16" fill="none" xmlns="http://www.w3.org/2000/svg">
                            <path d="M3.5 8C4.59202 9.04403 7.54398 10.3978 13.5068 9.93754M1.25349 5.39919C2.77722 0.413397 8.08911 0.79692 10.9673 1.24436C14.2687 1.71311 20.8969 3.82675 20.9985 8.53129C21.1255 14.412 13.1894 15.3069 10.0784 14.9233C6.96748 14.5398 -0.46071 13.0696 1.25349 5.39919Z" stroke="#838384" stroke-width="1.5" stroke-linecap="round"/>
                        </svg>
                    </footer>
                </main>
            </body>
        </html>"##,
    );

    Ok(webview_builder)
}
