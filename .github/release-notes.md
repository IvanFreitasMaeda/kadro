## Downloads

| Sistema | Arquivo |
| --- | --- |
| macOS (Apple Silicon, M1 ou mais novo, macOS 12.3+) | `Kadro-macOS-AppleSilicon.dmg` |
| Windows 10 (2004+) / 11, instalador | `Kadro-Windows-Setup.exe` |
| Windows, pacote MSI | `Kadro-Windows.msi` |

Macs com processador Intel não são suportados.

**macOS:** o app não tem assinatura da Apple. Depois de arrastar o Kadro para **Aplicativos**, rode no Terminal:

```sh
xattr -cr /Applications/Kadro.app
tccutil reset ScreenCapture io.github.ivanfreitasmaeda.kadro
```

**Windows:** se o SmartScreen avisar, clique em **Mais informações → Executar assim mesmo**.

Instruções completas, inclusive como limpar uma versão anterior, no [README](https://github.com/IvanFreitasMaeda/kadro#download).
