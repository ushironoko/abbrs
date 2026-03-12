---
name: release
description: abbrsのバージョン更新、GitHubリリース、crates.io publishを実行する。
user_invocable: true
disable-model-invocation: true
allowed-tools: Bash(git *), Bash(cargo test *), Bash(gh workflow run *), Bash(gh run *), Edit, Read, Grep, Glob
---

# Release Workflow

abbrsのリリース手順を実行するスキル。

## 引数

| 引数 | 必須 | 説明 |
|------|------|------|
| version | Yes | リリースするバージョン（例: 0.3.0） |

## 使用例

```
/release 0.3.0
```

## 実行フロー

### Phase 1: 事前確認

1. **引数検証**
   - バージョン引数が指定されていることを確認
   - semver形式（X.Y.Z）であることを確認

2. **現在のバージョン確認**
   ```bash
   grep '^version' Cargo.toml | head -1
   ```

3. **git状態確認**
   ```bash
   git status --porcelain
   git branch --show-current
   ```
   - mainブランチにいることを確認
   - ワーキングディレクトリがクリーンでない場合は警告して中断

4. **既存タグ確認**
   ```bash
   git tag --list 'v*' --sort=-version:refname | head -10
   ```
   - 同じバージョンのタグが既に存在する場合は中断

### Phase 2: バージョン更新

1. **Cargo.tomlのバージョン更新**
   - `version = "X.Y.Z"` を新しいバージョンに更新

2. **ビルド確認**（Cargo.lockは使用していないためcheck不要）
   ```bash
   cargo test
   ```
   - 失敗した場合は中断

### Phase 3: コミット & プッシュ

1. **変更をコミット**
   ```bash
   git add Cargo.toml
   git commit -m "chore: bump version to {version}"
   ```

2. **mainブランチにプッシュ**
   ```bash
   git push origin main
   ```

### Phase 4: リリースワークフロー起動

1. **バンプコミットのフルSHA取得**
   ```bash
   BUMP_SHA=$(git rev-parse HEAD)
   ```
   - **重要**: ワークフローは `bump_sha` がフルハッシュ（40文字）であることを前提とする。短縮SHAは使用不可。
   - ワークフローはこのSHAが `origin/main` のHEADであることを検証する。

2. **ワークフロー起動**
   ```bash
   gh workflow run release.yml -f version={version} -f bump_sha=$BUMP_SHA
   ```

3. **進捗確認**
   ```bash
   gh run list --workflow=release.yml --limit 1
   gh run watch <run-id>
   ```
   - ワークフローが validate → test → build → release → publish を自動実行
   - validate/test/build/release の失敗時はバンプコミットが自動revertされる

## 前提条件

- `gh` CLIがインストール・認証済み
- mainブランチにいること
- ワーキングディレクトリがクリーンであること
- リポジトリの trusted publishing (OIDC) が crates.io に設定済み

## ワークフローの処理内容

リリースワークフロー (`.github/workflows/release.yml`) は以下を実行する:

| ジョブ | 内容 |
|--------|------|
| validate | semver検証、bump_sha == origin/main HEAD検証、Cargo.tomlバージョン一致確認、タグ重複チェック、actions SHA pinチェック |
| test | `cargo test` + `cargo publish --dry-run` |
| build | 4プラットフォーム向けリリースビルド（x86_64-linux, aarch64-linux, x86_64-macos, aarch64-macos） |
| release | タグ作成・プッシュ、アーティファクトダウンロード、GitHub Release作成 |
| publish | OIDC経由でcrates.ioトークン取得、`cargo publish` |
| rollback | test/build/release失敗時に自動実行（コミットrevert、タグ・リリース削除） |

## エラー時のリカバリ

### validate/test/build/release の失敗
- 自動rollback（バンプコミットrevert + タグ/リリース削除）が実行される
- 原因を修正後、Phase 2 からやり直す

### publish のみ失敗（GitHub Release は成功済み）
- **推奨**: GitHub Actions UI から "Re-run failed jobs" で `publish` ジョブのみ再実行
- **手動**: ローカルで `cargo publish` を実行（`CARGO_REGISTRY_TOKEN` が必要）
- **注意**: `gh workflow run` での再実行はタグ重複バリデーションで失敗するため使用不可

### ネットワークエラー（ワークフロー起動自体の失敗）
- `gh workflow run release.yml -f version={version} -f bump_sha=$BUMP_SHA` で再試行
