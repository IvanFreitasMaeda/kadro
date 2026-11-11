# Kadro

Captura de tela simples e rápida para macOS e Windows, inspirada no Lightshot e no CleanShot.

Feito com [Tauri 2](https://v2.tauri.app) (Rust + TypeScript), para ser leve: o instalador fica na casa de poucos MB.

## O que já faz

- Fica na barra de menus (macOS) ou na bandeja (Windows).
- **Um clique** no ícone abre a captura. Clique direito mostra o menu.
- Selecione uma área arrastando, ou pegue a tela inteira.
- Ajuste a seleção depois: arraste para mover, use as alças para redimensionar.
- Anote antes de copiar: lápis, marca-texto, linha, seta, retângulo, elipse e texto, com cores, três espessuras e desfazer/refazer.
- Copie para a área de transferência ou salve em PNG numa pasta.
- Histórico na lateral: as últimas capturas aparecem em miniatura, e um clique copia de novo. Dá para salvar, abrir ou remover cada uma dali.
- Grave a tela inteira ou uma área em **MP4** ou **GIF**. Uma barrinha mostra o tempo e o botão de parar, e fica de fora do vídeo. A gravação vai para a pasta de capturas e aparece no histórico.
- Atalhos, pasta de capturas e tamanho do histórico configuráveis em Preferências.
- Funciona com vários monitores.

## Atalhos

| Ação | macOS | Windows |
| --- | --- | --- |
| Capturar área | `⌘ ⇧ 2` | `Ctrl Shift 2` |
| Capturar tela inteira | `⌘ ⇧ 1` | `Ctrl Shift 1` |
| Abrir/fechar o histórico | `⌘ ⇧ 0` | `Ctrl Shift 0` |
| Gravar a tela / parar a gravação | `⌘ ⇧ 9` | `Ctrl Shift 9` |

Dá para trocar os atalhos em **Preferências…**, no menu do ícone: clique no atalho e pressione a nova combinação.

Dentro da captura:

| Tecla | Ação |
| --- | --- |
| `Enter`, `Ctrl/⌘ C` ou duplo clique | Copiar e fechar |
| `Ctrl/⌘ S` | Salvar na pasta de capturas |
| `Ctrl/⌘ Shift S` | Salvar como… |
| `F` ou `Ctrl/⌘ A` | Selecionar a tela inteira |
| `Esc` ou clique direito | Cancelar |
| `P` `M` `L` `A` `R` `E` `T` | Lápis, marca-texto, linha, seta, retângulo, elipse, texto (de novo para soltar) |
| `Shift` enquanto desenha | Trava o ângulo da linha/seta, faz quadrado ou círculo |
| `Ctrl/⌘ Z` · `Ctrl/⌘ Shift Z` | Desfazer · refazer |

No texto, `Enter` confirma e `Shift Enter` quebra a linha.

Na gravação:

| Tecla | Ação |
| --- | --- |
| Arrastar | Escolher a área (ou `F` para a tela inteira) |
| `V` · `G` | Vídeo MP4 · GIF animado |
| `Enter` ou duplo clique | Começar a gravar |
| `⌘ ⇧ 9` / `Ctrl Shift 9`, ou clique no ícone do Kadro | Parar e salvar |

O botão de lixeira na barrinha descarta a gravação (clique duas vezes). O MP4 sai em 30 quadros por segundo, na resolução da tela (até 4K). O GIF sai em 15 quadros por segundo e no tamanho "normal" da tela (sem o dobro do Retina), com no máximo 1280 px de largura, para não ficar pesado.

As capturas são salvas em `Imagens/Kadro` por padrão. Dá para trocar a pasta em **Preferências…**.

No histórico:

| Tecla | Ação |
| --- | --- |
| Clique na miniatura ou `Enter` | Copiar (gravações são copiadas como arquivo, para colar num chat ou pasta) |
| `↑` `↓` | Escolher a captura |
| `Ctrl/⌘ S` · `Ctrl/⌘ Shift S` | Salvar na pasta (gravações: mostrar na pasta) · salvar como… |
| `Espaço` | Abrir no visualizador de imagens ou player |
| `Delete` | Remover do histórico |
| `Esc` ou clique fora | Fechar |

O histórico guarda as últimas 30 capturas por padrão (dá para mudar ou desativar em **Preferências…**). Elas ficam na pasta de dados do app, separadas da pasta de capturas: remover do histórico não apaga o que você salvou. As gravações não são duplicadas: o histórico só aponta para o arquivo na pasta de capturas.

## Instalar (versão de teste)

Os instaladores são gerados automaticamente a cada mudança, na aba **Actions** do GitHub (artefatos `kadro-macos-apple-silicon` e `kadro-windows-x64`). Macs com processador Intel não são suportados. No Mac é preciso o macOS 12.3 ou mais novo; no Windows, o Windows 10 (versão 2004) ou mais novo.

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
- `src-tauri/src/history.rs`: histórico de capturas (imagem ou gravação, e miniatura em disco).
- `src-tauri/src/record/`: gravação de tela. `macos.rs` usa ScreenCaptureKit + AVAssetWriter, `windows.rs` usa Windows Graphics Capture + Media Foundation (os dois codificam H.264 com o codificador do próprio sistema), `gif.rs` monta o GIF.
- `src-tauri/src/fileclip.rs`: copia um arquivo (a gravação) para a área de transferência.
- `src-tauri/src/gallery.rs`: janela lateral do histórico.
- `src-tauri/src/settings.rs`: janela de preferências.
- `src-tauri/src/config.rs`: configurações salvas (atalhos, pasta de capturas e tamanho do histórico).
- `src/overlay.ts`: interface de seleção e anotação.
- `src/annotate.ts`: formas das anotações e como desenhá-las.
- `src/gallery.ts`: interface do histórico.
- `src/record.ts`: barrinha de controle da gravação.
- `src/settings.ts`: interface das preferências.

## Próximos passos

1. ~~Captura de imagem~~
2. ~~Anotações: lápis, pincel, setas, formas e texto~~
3. ~~Histórico em galeria na lateral~~
4. ~~Gravação de tela em MP4 e GIF~~
5. Instaladores e releases públicas

## Licença

MIT
