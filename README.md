# Kadro

Captura de tela simples e rápida para macOS e Windows, inspirada no Lightshot e no CleanShot.

Feito com [Tauri 2](https://v2.tauri.app) (Rust + TypeScript), para ser leve: o instalador fica na casa de poucos MB.

## O que já faz

- Fica na barra de menus (macOS) ou na bandeja (Windows).
- **Um clique** no ícone abre a captura. Clique direito mostra o menu.
- Selecione uma área arrastando, ou pegue a tela inteira.
- Ajuste a seleção depois: arraste para mover, use as alças para redimensionar.
- Copie para a área de transferência ou salve em PNG numa pasta.
- Atalhos e pasta de capturas configuráveis em Preferências.
- Funciona com vários monitores.

## Atalhos

| Ação | macOS | Windows |
| --- | --- | --- |
| Capturar área | `⌘ ⇧ 2` | `Ctrl Shift 2` |
| Capturar tela inteira | `⌘ ⇧ 1` | `Ctrl Shift 1` |

Dá para trocar os dois atalhos em **Preferências…**, no menu do ícone: clique no atalho e pressione a nova combinação.

Dentro da captura:

| Tecla | Ação |
| --- | --- |
| `Enter`, `Ctrl/⌘ C` ou duplo clique | Copiar e fechar |
| `Ctrl/⌘ S` | Salvar na pasta de capturas |
| `Ctrl/⌘ Shift S` | Salvar como… |
| `F` ou `Ctrl/⌘ A` | Selecionar a tela inteira |
| `Esc` ou clique direito | Cancelar |

As capturas são salvas em `Imagens/Kadro` por padrão. Dá para trocar a pasta em **Preferências…**.

## Instalar (versão de teste)

Os instaladores são gerados automaticamente a cada mudança, na aba **Actions** do GitHub (artefatos `kadro-macos-apple-silicon`, `kadro-macos-intel` e `kadro-windows-x64`).

**macOS:** o app ainda não tem assinatura da Apple, então o Mac bloqueia na primeira vez. Arraste o Kadro para **Aplicativos** e rode no Terminal:

```sh
xattr -cr /Applications/Kadro.app
```

Se ainda aparecer aviso, vá em **Ajustes do Sistema → Privacidade e Segurança** e clique em **Abrir Mesmo Assim**. O Kadro não abre janela: ele aparece como um ícone na barra de menus. Na primeira captura o macOS pede a permissão de **Gravação de Tela**. Depois de liberar, feche o Kadro pelo menu do ícone e abra de novo.

Enquanto o app não tiver assinatura da Apple, cada versão nova é vista pelo macOS como outro app. Se a permissão aparecer marcada mas a captura não funcionar, rode `tccutil reset ScreenCapture io.github.ivanfreitasmaeda.kadro`, abra o Kadro e libere de novo.

**Windows:** o SmartScreen pode avisar que o app é desconhecido. Clique em **Mais informações → Executar assim mesmo**.

## Desenvolvimento

Requisitos: [Node.js 22+](https://nodejs.org), [Rust](https://rustup.rs) e os [pré-requisitos do Tauri](https://v2.tauri.app/start/prerequisites/) do seu sistema.

```sh
npm install
npm run tauri dev     # roda em modo desenvolvimento
npm run tauri build   # gera o instalador em src-tauri/target/release/bundle
```

Estrutura:

- `src-tauri/src/capture.rs`: captura dos monitores, janelas de seleção, recorte, copiar e salvar.
- `src-tauri/src/tray.rs`: ícone e menu da bandeja.
- `src-tauri/src/shortcuts.rs`: atalhos globais configuráveis.
- `src-tauri/src/settings.rs`: janela de preferências.
- `src-tauri/src/config.rs`: configurações salvas (atalhos e pasta de capturas).
- `src/overlay.ts`: interface de seleção.
- `src/settings.ts`: interface das preferências.

## Próximos passos

1. ~~Captura de imagem~~
2. Anotações: lápis, pincel, setas, formas e texto
3. Histórico em galeria na lateral
4. Gravação de tela em MP4 e GIF
5. Instaladores e releases públicas

## Licença

MIT
