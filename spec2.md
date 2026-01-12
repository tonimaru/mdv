# mdv 連携拡張：実装指示書

## 1. サーバー側（Rust）への追加実装

既存の `mdv` に、外部制御のための「口」を 2 つ追加してください。

### A. 通信プロトコルとエンドポイント

ブラウザ側には **WebSocket** を、エディタ側からは **HTTP** をインターフェースとして提供します。

1. **WebSocket (`/ws`)**:
* ブラウザがページを読み込んだ際に接続。
* サーバーから送られてくる JSON 命令を待機。


2. **Remote Control API (`GET /api/remote/navigate`)**:
* クエリパラメータ: `?path=relative/path/to.md`
* 処理: 受信後、現在 `/ws` に接続中の全クライアントへ `{ "type": "navigate", "url": "/relative/path/to.md" }` をブロードキャスト。


3. **Remote Control API (`GET /api/remote/scroll`)**:
* クエリパラメータ: `?percent=0-100`
* 処理: 全クライアントへ `{ "type": "scroll", "percent": X }` をブロードキャスト。



### B. ブラウザ側 JavaScript (既存HTMLテンプレートへ埋め込み)

各プレビュー画面に以下の JS を注入してください。

```javascript
const ws = new WebSocket(`ws://${location.host}/ws`);
let ignoreScroll = false;

ws.onmessage = (e) => {
    const data = JSON.parse(e.data);
    if (data.type === 'navigate') {
        window.location.href = data.url; // ページ移動
    } else if (data.type === 'scroll' && !ignoreScroll) {
        const target = (document.documentElement.scrollHeight - window.innerHeight) * (data.percent / 100);
        window.scrollTo({ top: target, behavior: 'smooth' });
    }
};

// ユーザーが自らスクロールしたら、一時的にエディタからの追従を無視
window.onwheel = () => {
    ignoreScroll = true;
    setTimeout(() => { ignoreScroll = false; }, 2000); // 2秒後に復帰
    // オプション: サーバーに "user_scrolled" イベントを投げてVim側のフラグを完全に切ることも可能
};

```

---

## 2. エディタ側（Vim/Neovim 共通スクリプト）

以下の VimScript を作成し、`.vimrc` またはプラグインとしてロードします。

```vim
" --- グローバル設定 ---
let g:mdv_port_file = '.mdv_port'
let g:mdv_sync_scroll = 1

" --- ユーティリティ: 非同期ジョブ実行 ---
function! s:mdv_run(cmd)
    if has('nvim')
        call jobstart(a:cmd)
    elseif has('job')
        call job_start(a:cmd)
    endif
endfunction

" --- 機能1: mdvの起動/アタッチ ---
function! MdvOpen()
    let l:root = getcwd()
    let l:port = 8080 " デフォルト。本来は .mdv_port から読み取る処理を推奨

    " mdv が起動していなければ起動 (例: mdv . --port 8080)
    if !filereadable(g:mdv_port_file)
        call s:mdv_run(['mdv', l:root, '--port', string(l:port)])
        sleep 500m " 起動待ち
    endif

    " 現在のファイルをブラウザで開く (OSごとのopenコマンド)
    let l:url = "http://localhost:" . l:port . "/" . expand('%')
    if has('mac') | call s:mdv_run(['open', l:url])
    elseif has('unix') | call s:mdv_run(['xdg-open', l:url])
    endif
endfunction

" --- 機能2: ページ同期 ---
function! s:MdvSyncPage()
    let l:port = 8080 " 同上
    let l:path = expand('%')
    call s:mdv_run(['curl', '-s', "http://localhost:".l:port."/api/remote/navigate?path=".l:path])
endfunction

" --- 機能3: スクロール同期 ---
function! s:MdvSyncScroll()
    if g:mdv_sync_scroll == 0 | return | endif
    let l:port = 8080
    " ウィンドウの最上行がファイル全体の何%か
    let l:percent = (line('w0') * 100) / line('$')
    call s:mdv_run(['curl', '-s', "http://localhost:".l:port."/api/remote/scroll?percent=".l:percent])
endfunction

" --- 自動コマンド登録 ---
augroup MDV_Sync
    autocmd!
    autocmd BufEnter *.md call s:MdvSyncPage()
    autocmd CursorMoved,CursorMovedI *.md call s:MdvSyncScroll()
augroup END

" --- コマンド定義 ---
command! MdvOpen call MdvOpen()
command! MdvScrollToggle let g:mdv_sync_scroll = !g:mdv_sync_scroll | echo "MDV Scroll Sync: " . (g:mdv_sync_scroll ? "ON" : "OFF")

```
