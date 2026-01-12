# ソフトウェア仕様書: Markdown Directory Viewer (MDV)

## 1. 概要

指定されたローカルディレクトリをルートとし、Markdownファイルのレンダリングおよびディレクトリ構造のブラウジングを可能にするWebベースのプレビューツール。

## 2. 技術スタック

* **Language:** Rust
* **Web Framework:** `axum` (Tokioベースの高速フレームワーク)
* **Markdown Engine:** `pulldown-cmark` (GitHub Flavored Markdown互換)
* **Syntax Highlighting:** `syntect` または `prism.js` (クライアントサイド)
* **Styling:** Tailwind CSS (GitHub風のデザイン再現用)
* **Template Engine:** `askama` (コンパイル時に型チェックされるテンプレート)

---

## 3. 機能要件

### A. サーバー機能

1. **静的ファイルサービング:** 指定ディレクトリ内の画像等のアセットを表示可能にする。
2. **動的レンダリング:** `.md`ファイルへのリクエストに対し、HTMLを生成してレスポンスする。
3. **ディレクトリリスティング:** ディレクトリへのリクエストに対し、ファイル/フォルダ一覧画面を生成する。

### B. UI/UX (GitHubスタイル)

1. **パンくずリスト (Breadcrumbs):** * 画面上部に `root / path / to / file.md` の形式で表示。各要素はクリック可能なリンク。
2. **ファイルビューアー:**
* MarkdownをHTMLに変換。
* GFM (GitHub Flavored Markdown) のテーブル、タスクリスト、コードブロックに対応。


3. **ディレクトリビューアー:**
* フォルダとファイルをアイコン（SVG）付きで一覧表示。
* ファイルサイズや最終更新日時を表示（オプション）。



---

## 4. ルーティング設計

| パス | 処理内容 | 表示形式 |
| --- | --- | --- |
| `/` | ルートディレクトリの内容を表示 | ディレクトリ一覧 |
| `/*path` (ディレクトリ) | 該当パス内のファイル/フォルダ一覧を表示 | ディレクトリ一覧 |
| `/*path.md` | MarkdownをパースしてHTMLとして表示 | プレビュー画面 |
| `/*path.(jpg|png|...)` | ファイルをそのまま配信 | バイナリデータ |

---

## 5. 画面設計（ワイヤーフレーム定義）

### 共通コンポーネント

* **Header:** ロゴと現在のパス（パンくずリスト）。
* **Container:** 中央寄せ（max-width: 1012px ※GitHub標準）。

### プレビュー画面

1. **File Header:** ファイル名、ファイルサイズ、Rawボタン。
2. **Markdown Body:** `markdown-body` クラス（github-markdown-css）を適用したレンダリング領域。

### ディレクトリ一覧画面

1. **File List Table:**
* Icon | Name | Last Commit (Optional) | Date
* 最上部に `..` (Parent Directory) へのリンク。



---

## 6. 実装上の重要ポイント

### Markdownの変換処理

Rustの `pulldown-cmark` を使用して変換します。

```rust
let mut options = Options::empty();
options.insert(Options::ENABLE_TABLES);
options.insert(Options::ENABLE_FOOTNOTES);
options.insert(Options::ENABLE_STRIKETHROUGH);
options.insert(Options::ENABLE_TASKLISTS);

let parser = Parser::new_ext(&markdown_input, options);
let mut html_output = String::new();
html::push_html(&mut html_output, parser);

```

### セキュリティ

* **ディレクトリトラバーサル対策:** リクエストされたパスが、起動時に指定したルートディレクトリの外を参照していないかを厳密にチェックする必要があります。

---

## 7. 開発ステップ（タスクリスト）

1. [ ] **Project Setup:** `cargo init` および依存関係（`axum`, `tokio`, `pulldown-cmark`, `askama`, `tower-http`）の追加。
2. [ ] **File System Logic:** パスを受け取り、それがファイルかディレクトリか判定するユーティリティ関数の作成。
3. [ ] **Template Implementation:** `github-markdown-css` を取り込んだHTMLテンプレートの作成。
4. [ ] **Router Implementation:** Axumによるルーティングとハンドラの実装。
5. [ ] **Breadcrumb Logic:** パス文字列を分割し、リンク付きリストに変換するロジックの実装。
6. [ ] **Styling:** Tailwind CSSを用いたGitHub風UIの微調整。
