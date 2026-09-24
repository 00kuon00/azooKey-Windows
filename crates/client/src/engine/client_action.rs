use super::input_mode::InputMode;

#[derive(Debug, PartialEq)]
pub enum ClientAction {
    StartComposition,
    EndComposition,

    AppendText(String),
    RemoveText,
    ShrinkText(String),
    // 表示中の候補を確定したことをサーバへ伝える（学習）。EndComposition / ShrinkText の前に置く
    CommitCandidate,
    // 表示中の候補の学習だけを忘れさせ、候補を取り直す（候補選択中の Ctrl+Delete）
    ForgetCandidate,

    SetTextWithType(SetTextType),

    MoveCursor(i32),
    SetSelection(SetSelectionType),
    // 最初の文節の読みを増減する（Shift+←→）。値は増やす文字数
    MoveSegmentBoundary(i32),

    SetIMEMode(InputMode),
}

#[derive(Debug, PartialEq)]
pub enum SetSelectionType {
    Up,
    Down,
    Number(i32),
}

#[derive(Debug, PartialEq)]
pub enum SetTextType {
    Hiragana,     // F6
    Katakana,     // F7
    HalfKatakana, // F8
    FullLatin,    // F9
    HalfLatin,    // F10
}
