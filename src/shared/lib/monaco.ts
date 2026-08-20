import * as monaco from 'monaco-editor/esm/vs/editor/editor.api.js'
import 'monaco-editor/esm/vs/basic-languages/python/python.contribution'
import 'monaco-editor/esm/vs/editor/browser/coreCommands'
import 'monaco-editor/esm/vs/editor/browser/widget/codeEditor/codeEditorWidget'
import 'monaco-editor/esm/vs/editor/browser/widget/diffEditor/diffEditor.contribution'
import 'monaco-editor/esm/vs/editor/contrib/anchorSelect/browser/anchorSelect'
import 'monaco-editor/esm/vs/editor/contrib/bracketMatching/browser/bracketMatching'
import 'monaco-editor/esm/vs/editor/contrib/caretOperations/browser/caretOperations'
import 'monaco-editor/esm/vs/editor/contrib/caretOperations/browser/transpose'
import 'monaco-editor/esm/vs/editor/contrib/clipboard/browser/clipboard'
import 'monaco-editor/esm/vs/editor/contrib/codeAction/browser/codeActionContributions'
import 'monaco-editor/esm/vs/editor/contrib/codelens/browser/codelensController'
import 'monaco-editor/esm/vs/editor/contrib/colorPicker/browser/colorPickerContribution'
import 'monaco-editor/esm/vs/editor/contrib/comment/browser/comment'
import 'monaco-editor/esm/vs/editor/contrib/contextmenu/browser/contextmenu'
import 'monaco-editor/esm/vs/editor/contrib/cursorUndo/browser/cursorUndo'
import 'monaco-editor/esm/vs/editor/contrib/dnd/browser/dnd'
import 'monaco-editor/esm/vs/editor/contrib/dropOrPasteInto/browser/copyPasteContribution'
import 'monaco-editor/esm/vs/editor/contrib/dropOrPasteInto/browser/dropIntoEditorContribution'
import 'monaco-editor/esm/vs/editor/contrib/find/browser/findController'
import 'monaco-editor/esm/vs/editor/contrib/folding/browser/folding'
import 'monaco-editor/esm/vs/editor/contrib/fontZoom/browser/fontZoom'
import 'monaco-editor/esm/vs/editor/contrib/format/browser/formatActions'
import 'monaco-editor/esm/vs/editor/contrib/documentSymbols/browser/documentSymbols'
import 'monaco-editor/esm/vs/editor/contrib/inlineCompletions/browser/inlineCompletions.contribution'
import 'monaco-editor/esm/vs/editor/contrib/inlineProgress/browser/inlineProgress'
import 'monaco-editor/esm/vs/editor/contrib/gotoSymbol/browser/goToCommands'
import 'monaco-editor/esm/vs/editor/contrib/gotoSymbol/browser/link/goToDefinitionAtPosition'
import 'monaco-editor/esm/vs/editor/contrib/gotoError/browser/gotoError'
import 'monaco-editor/esm/vs/editor/contrib/gpu/browser/gpuActions'
import 'monaco-editor/esm/vs/editor/contrib/hover/browser/hoverContribution'
import 'monaco-editor/esm/vs/editor/contrib/indentation/browser/indentation'
import 'monaco-editor/esm/vs/editor/contrib/inlayHints/browser/inlayHintsContribution'
import 'monaco-editor/esm/vs/editor/contrib/inPlaceReplace/browser/inPlaceReplace'
import 'monaco-editor/esm/vs/editor/contrib/insertFinalNewLine/browser/insertFinalNewLine'
import 'monaco-editor/esm/vs/editor/contrib/lineSelection/browser/lineSelection'
import 'monaco-editor/esm/vs/editor/contrib/linesOperations/browser/linesOperations'
import 'monaco-editor/esm/vs/editor/contrib/linkedEditing/browser/linkedEditing'
import 'monaco-editor/esm/vs/editor/contrib/links/browser/links'
import 'monaco-editor/esm/vs/editor/contrib/longLinesHelper/browser/longLinesHelper'
import 'monaco-editor/esm/vs/editor/contrib/middleScroll/browser/middleScroll.contribution'
import 'monaco-editor/esm/vs/editor/contrib/multicursor/browser/multicursor'
import 'monaco-editor/esm/vs/editor/contrib/parameterHints/browser/parameterHints'
import 'monaco-editor/esm/vs/editor/contrib/placeholderText/browser/placeholderText.contribution'
import 'monaco-editor/esm/vs/editor/contrib/rename/browser/rename'
import 'monaco-editor/esm/vs/editor/contrib/sectionHeaders/browser/sectionHeaders'
import 'monaco-editor/esm/vs/editor/contrib/semanticTokens/browser/documentSemanticTokens'
import 'monaco-editor/esm/vs/editor/contrib/semanticTokens/browser/viewportSemanticTokens'
import 'monaco-editor/esm/vs/editor/contrib/smartSelect/browser/smartSelect'
import 'monaco-editor/esm/vs/editor/contrib/snippet/browser/snippetController2'
import 'monaco-editor/esm/vs/editor/contrib/stickyScroll/browser/stickyScrollContribution'
import 'monaco-editor/esm/vs/editor/contrib/suggest/browser/suggestController'
import 'monaco-editor/esm/vs/editor/contrib/suggest/browser/suggestInlineCompletions'
import 'monaco-editor/esm/vs/editor/contrib/tokenization/browser/tokenization'
import 'monaco-editor/esm/vs/editor/contrib/toggleTabFocusMode/browser/toggleTabFocusMode'
import 'monaco-editor/esm/vs/editor/contrib/unicodeHighlighter/browser/unicodeHighlighter'
import 'monaco-editor/esm/vs/editor/contrib/unusualLineTerminators/browser/unusualLineTerminators'
import 'monaco-editor/esm/vs/editor/contrib/wordHighlighter/browser/wordHighlighter'
import 'monaco-editor/esm/vs/editor/contrib/wordOperations/browser/wordOperations'
import 'monaco-editor/esm/vs/editor/contrib/wordPartOperations/browser/wordPartOperations'
import 'monaco-editor/esm/vs/editor/contrib/readOnlyMessage/browser/contribution'
import 'monaco-editor/esm/vs/editor/contrib/diffEditorBreadcrumbs/browser/contribution'
import 'monaco-editor/esm/vs/editor/contrib/floatingMenu/browser/floatingMenu.contribution'
import 'monaco-editor/esm/vs/editor/common/standaloneStrings'
import 'monaco-editor/esm/vs/base/browser/ui/codicons/codicon/codicon.css'
import 'monaco-editor/esm/vs/base/browser/ui/codicons/codicon/codicon-modifiers.css'
import 'monaco-editor/esm/vs/editor/standalone/browser/iPadShowKeyboard/iPadShowKeyboard'
import 'monaco-editor/esm/vs/editor/standalone/browser/inspectTokens/inspectTokens'
import 'monaco-editor/esm/vs/editor/standalone/browser/quickAccess/standaloneHelpQuickAccess'
import 'monaco-editor/esm/vs/editor/standalone/browser/quickAccess/standaloneGotoLineQuickAccess'
import 'monaco-editor/esm/vs/editor/standalone/browser/quickAccess/standaloneGotoSymbolQuickAccess'
import 'monaco-editor/esm/vs/editor/standalone/browser/quickAccess/standaloneCommandsQuickAccess'
import 'monaco-editor/esm/vs/editor/standalone/browser/referenceSearch/standaloneReferenceSearch'
import 'monaco-editor/esm/vs/editor/standalone/browser/toggleHighContrast/toggleHighContrast'
import EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker&inline'

declare global {
  interface Window {
    MonacoEnvironment?: monaco.Environment
  }
}

// Python only needs the core editor worker (its grammar is Monarch, main-thread).
self.MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
}

monaco.editor.defineTheme('wms-dark', {
  base: 'vs-dark',
  inherit: true,
  rules: [
    { token: 'log.timestamp', foreground: '6A737D' },
    { token: 'log.category', foreground: '56C8D8' },
    { token: 'log.info', foreground: '58A6FF', fontStyle: 'bold' },
    { token: 'log.debug', foreground: '7D8799', fontStyle: 'bold' },
    { token: 'log.notice', foreground: 'B392F0', fontStyle: 'bold' },
    { token: 'log.warning', foreground: 'E3B341', fontStyle: 'bold' },
    { token: 'log.error', foreground: 'F06D6D', fontStyle: 'bold' },
    { token: 'log.critical', foreground: 'FF7B72', fontStyle: 'bold' },
    { token: 'log.input', foreground: '7DD3FC', fontStyle: 'bold' },
    { token: 'log.traceback', foreground: 'F06D6D' },
  ],
  colors: {
    'editor.background': '#0E1116',
    'editor.foreground': '#C9D3DF',
    'editorLineNumber.foreground': '#4C586A',
    'editorCursor.foreground': '#3FB9B0',
    'editorGutter.background': '#0E1116',
    'editor.lineHighlightBackground': '#151A21',
    'editorWidget.background': '#151A21',
    'editorWidget.border': '#232B36',
    'editor.findMatchBackground': '#3FB9B066',
    'editor.findMatchHighlightBackground': '#3FB9B033',
    'editor.findRangeHighlightBackground': '#3FB9B01F',
  },
})

export { monaco }
