import { normalizeProjectViewTemplates } from './project-views';
import { normalizeProjectWebsiteSettings, normalizeProjectWebsiteVisibility } from './project-websites';
import { normalizeContentThemeSetting } from '../appearance';
import { clampAgentManagerZoomPercent, clampSidebarThemeSetting } from '../session-grid-contract-session';
import { normalizeSessionChatTheme } from '../session-chat';
import { clampCompletionSoundSetting } from '../completion-sound';
import { getTerminalFontFamilyForPreset } from '../terminal-font-preset';
import { normalizeghostexHotkeySettings } from '../ghostex-hotkeys';
import {
  normalizeCustomWorkspaceOpenTargets,
  normalizeWorkspaceOpenTargetAvailability,
  normalizeWorkspaceOpenTargetHiddenIds,
} from '../workspace-open-targets';
import { normalizePetId } from '../pets';
import { normalizeSidebarSessionTagListItems } from '../session-tags';
import { DEFAULT_ghostex_SETTINGS } from './defaults';
import { normalizeGhostexCustomViews } from './custom-views';
import { normalizeTitlebarViewOrder } from './titlebar-view-order';
import { normalizeGhostexViewScopes } from './view-scopes';
import { normalizeDiagnosticLoggingSettings } from './diagnostic-logging';
import { COMMANDS_PANEL_AUTO_MINIMIZE_DELAY_OPTIONS } from './option-tables';
import { SIDEBAR_SETTINGS_PRESET_SETTINGS } from './presets';
import { clampNumber, isRecord, readBoolean, readNumber, readString } from './primitives';
import { normalizeRemoteMachineSettings } from './remote-machines';
import {
  normalizeCustomSessionTitleGenerationCommand,
  normalizeSessionTitleGenerationAgent,
} from './session-title-generation';
import { normalizeSettingsModalNavigationState } from './settings-modal-navigation';
import { normalizeTerminalDevServerIgnoredPortRules } from './terminal-dev-servers';
import {
  clampSidebarTitlebarBackgroundDarknessPercent,
  clampSidebarTitlebarLightBackgroundLightnessPercent,
  getSidebarTitlebarBackgroundDarknessForColor,
  getSidebarTitlebarBackgroundForDarkness,
  getSidebarTitlebarForegroundForBackground,
  getSidebarTitlebarLightBackgroundForLightness,
  normalizeDarkThemePreset,
  normalizeLightThemePreset,
  normalizeSidebarTitlebarHexColor,
  readThemeContrastPoints,
  resolveDarkChromeControls,
  resolveLightChromeControls,
} from './titlebar-color';
import {
  type SidebarSettingsPresetId,
  clampCommandsPanelDefaultHeightPx,
  clampProjectSessionListCollapsedCount,
  clampSessionChatTranscriptWidthPercent,
  clampSessionChatZoomPercent,
  clampTerminalViewWidthPercent,
  clampSidebarCollapseAnimationDurationMs,
  clampSidebarDefaultWidthPx,
  clampProjectSwitchKeepAliveMinutes,
  clampSidebarTooltipDelayMs,
  clampTerminalPanePaddingPx,
  clampWindowGlassSidebarOpacityPercent,
  type ghostexSettings,
} from './types';
import {
  MIN_GHOSTTY_MOUSE_SCROLL_MULTIPLIER,
  MAX_GHOSTTY_MOUSE_SCROLL_MULTIPLIER,
  MIN_GHOSTTY_SCROLLBACK_LIMIT_MB,
  MAX_GHOSTTY_SCROLLBACK_LIMIT_MB,
  normalizeTitlebarProjectSelectionMap,
  normalizeTerminalViewWidthMode,
  normalizeManageAdditionalDocsFolders,
  normalizeGlobalWorktreeCommand,
  normalizeGlobalBeadsDisplayKey,
  normalizeGlobalBeadsDirectory,
  normalizeGlobalDocsDirectory,
  normalizeSessionCardHoverButtonsSetting,
  normalizeTerminalCursorStyle,
  normalizeWindowsWslDistribution,
  normalizeDefaultEditorCommand,
  normalizeCustomDefaultEditorCommand,
  normalizeAppIconSourceId,
  normalizeDefaultPromptAgentId,
  normalizeAgentboxDefaultLocation,
  normalizeWebLinkOpenTarget,
  normalizeMediaFileOpenTarget,
  normalizeChatFileOpenView,
  normalizeCommandsPanelSide,
  normalizeWindowGlassMode,
  normalizeWindowGlassSource,
  normalizeWindowGlassVideoPath,
  normalizeWindowGlassLiveSlot,
  normalizeWindowGlassLiveBrightness,
  normalizeWindowGlassBlurRadius,
  normalizeWindowGlassLiveSpeed,
  normalizeWindowGlassImagePlacement,
  normalizePanelAnimationSpeed,
  normalizeSidebarSpaceSwitchBehavior,
  normalizeSidebarVisibilityMemory,
  normalizePreferredAgentInterface,
  normalizePreferredAgentInterfaceOverrides,
  normalizeKeepAwakeDurationMinutes,
  normalizeAutoSleepIdleMinutes,
  normalizeKeepAwakeBatteryThresholdPercent,
  normalizeCompletionSoundPreference,
  normalizePromptEditorBackend,
  normalizeGhosttyTheme,
  normalizeGhosttyFontFamily,
  normalizeGhosttyCopyOnSelect,
  normalizeGhosttyConfirmCloseSurface,
  normalizeGhosttyScrollbar,
  normalizeTerminalBackgroundImageFit,
  normalizePortlessProtocol,
  normalizeWindowGlassWorkAreaTint,
  normalizeTerminalBackgroundMode,
  normalizeTerminalBackgroundSetting,
} from './normalize-fields';
export {
  normalizeManageAdditionalDocsFolders,
  normalizeGlobalWorktreeCommand,
  normalizeGlobalBeadsDisplayKey,
  normalizeGlobalBeadsDirectory,
  normalizeGlobalDocsDirectory,
  getDefaultEditorCommandForSettings,
} from './normalize-fields';

export function normalizeghostexSettings(candidate: unknown): ghostexSettings {
  const source = isRecord(candidate) ? candidate : {};
  const promptEditorBackend = normalizePromptEditorBackend(source);
  const webLinkOpenTarget = normalizeWebLinkOpenTarget(source);
  const markdownFileOpenView = normalizeChatFileOpenView(source.markdownFileOpenView);
  const htmlFileOpenView = normalizeChatFileOpenView(source.htmlFileOpenView);
  const rawLegacyCustomSidebarTitlebarBackgroundColor = source.customSidebarTitlebarBackgroundColor;
  const hasValidLegacyCustomSidebarTitlebarBackgroundColor =
    typeof rawLegacyCustomSidebarTitlebarBackgroundColor === 'string' &&
    /^#[0-9a-f]{6}$/u.test(rawLegacyCustomSidebarTitlebarBackgroundColor.trim().toLowerCase());
  const legacyCustomSidebarTitlebarBackgroundColor = normalizeSidebarTitlebarHexColor(
    readString(
      source,
      'customSidebarTitlebarBackgroundColor',
      DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundColor
    ),
    DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundColor
  );
  /**
   * CDXC:Theming 2026-06-16-14:28:
   * Missing Settings should use the explicit 95 contrast default instead of
   * reverse-mapping the default background hex, because that reverse mapping
   * cannot exactly invert the slider's channel curve. Only valid legacy saved
   * background colors should continue to seed the slider during migration.
   */
  const customSidebarTitlebarBackgroundDarknessFallback = hasValidLegacyCustomSidebarTitlebarBackgroundColor
    ? getSidebarTitlebarBackgroundDarknessForColor(legacyCustomSidebarTitlebarBackgroundColor)
    : DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundDarknessPercent;
  const customSidebarTitlebarBackgroundDarknessPercent = clampSidebarTitlebarBackgroundDarknessPercent(
    readNumber(
      source,
      'customSidebarTitlebarBackgroundDarknessPercent',
      customSidebarTitlebarBackgroundDarknessFallback
    )
  );
  const customSidebarTitlebarBackgroundTintColor = normalizeSidebarTitlebarHexColor(
    readString(
      source,
      'customSidebarTitlebarBackgroundTintColor',
      DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundTintColor
    ),
    DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundTintColor
  );
  /**
   * CDXC:Theming 2026-09-22 WHY:
   * A settings file saved before the preset dropdowns existed has no preset key. If its contrast or
   * tint differs from the shipped default, the user tuned them, so the dark dropdown migrates to Custom
   * and their chrome is unchanged; otherwise it lands on the Gray preset that resolves to the same color.
   */
  const darkThemePreset =
    normalizeDarkThemePreset(source.darkThemePreset) ??
    (customSidebarTitlebarBackgroundDarknessPercent !==
      DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundDarknessPercent ||
    customSidebarTitlebarBackgroundTintColor !== DEFAULT_ghostex_SETTINGS.customSidebarTitlebarBackgroundTintColor
      ? 'custom'
      : DEFAULT_ghostex_SETTINGS.darkThemePreset);
  const themeSidebarContrast = readThemeContrastPoints(source, 'themeSidebarContrast');
  const themeWorkAreaContrast = readThemeContrastPoints(source, 'themeWorkAreaContrast');
  const darkChromeControls = resolveDarkChromeControls({
    themeSidebarContrast,
    darkThemePreset,
    customSidebarTitlebarBackgroundDarknessPercent,
    customSidebarTitlebarBackgroundTintColor,
  });
  const customSidebarTitlebarBackgroundColor = getSidebarTitlebarBackgroundForDarkness(
    darkChromeControls.darknessPercent,
    darkChromeControls.tintColor
  );
  const customSidebarTitlebarLightBackgroundLightnessPercent = clampSidebarTitlebarLightBackgroundLightnessPercent(
    readNumber(
      source,
      'customSidebarTitlebarLightBackgroundLightnessPercent',
      DEFAULT_ghostex_SETTINGS.customSidebarTitlebarLightBackgroundLightnessPercent
    )
  );
  const customSidebarTitlebarLightBackgroundTintColor = normalizeSidebarTitlebarHexColor(
    readString(
      source,
      'customSidebarTitlebarLightBackgroundTintColor',
      DEFAULT_ghostex_SETTINGS.customSidebarTitlebarLightBackgroundTintColor
    ),
    DEFAULT_ghostex_SETTINGS.customSidebarTitlebarLightBackgroundTintColor
  );
  const lightThemePreset =
    normalizeLightThemePreset(source.lightThemePreset) ?? DEFAULT_ghostex_SETTINGS.lightThemePreset;
  const lightChromeControls = resolveLightChromeControls({
    themeSidebarContrast,
    lightThemePreset,
    customSidebarTitlebarLightBackgroundLightnessPercent,
    customSidebarTitlebarLightBackgroundTintColor,
  });
  const customSidebarTitlebarLightBackgroundColor = getSidebarTitlebarLightBackgroundForLightness(
    lightChromeControls.lightnessPercent,
    lightChromeControls.tintColor
  );
  return {
    actionCompletionSound: clampCompletionSoundSetting(
      readString(source, 'actionCompletionSound', DEFAULT_ghostex_SETTINGS.actionCompletionSound)
    ),
    gpuiTitlebarActionCommandByProject: normalizeTitlebarProjectSelectionMap(source.gpuiTitlebarActionCommandByProject),
    gpuiTitlebarOpenTargetByProject: normalizeTitlebarProjectSelectionMap(source.gpuiTitlebarOpenTargetByProject),
    ghostexCaptureEnabled: readBoolean(source, 'ghostexCaptureEnabled', DEFAULT_ghostex_SETTINGS.ghostexCaptureEnabled),
    ghostexCaptureSwitchToSession: readBoolean(
      source,
      'ghostexCaptureSwitchToSession',
      DEFAULT_ghostex_SETTINGS.ghostexCaptureSwitchToSession
    ),
    agentboxDefaultLocation: normalizeAgentboxDefaultLocation(
      readString(source, 'agentboxDefaultLocation', DEFAULT_ghostex_SETTINGS.agentboxDefaultLocation)
    ),
    agentAcceptAllEnabled: readBoolean(source, 'agentAcceptAllEnabled', DEFAULT_ghostex_SETTINGS.agentAcceptAllEnabled),
    agentManagerZoomPercent: clampAgentManagerZoomPercent(
      readNumber(source, 'agentManagerZoomPercent', DEFAULT_ghostex_SETTINGS.agentManagerZoomPercent)
    ),
    /**
     * CDXC:AgentLauncher 2026-05-28-07:15:
     * Keep the selected default prompt agent as a plain agent id so built-in,
     * reordered, hidden-restored, and custom agents can all be selected without
     * coupling settings normalization to the runtime agent registry.
     */
    defaultPromptAgentId: normalizeDefaultPromptAgentId(
      readString(source, 'defaultPromptAgentId', DEFAULT_ghostex_SETTINGS.defaultPromptAgentId)
    ),
    sessionTitleGenerationAgent: normalizeSessionTitleGenerationAgent(
      readString(source, 'sessionTitleGenerationAgent', DEFAULT_ghostex_SETTINGS.sessionTitleGenerationAgent)
    ),
    customSessionTitleGenerationCommand: normalizeCustomSessionTitleGenerationCommand(
      readString(
        source,
        'customSessionTitleGenerationCommand',
        DEFAULT_ghostex_SETTINGS.customSessionTitleGenerationCommand
      )
    ),
    webLinkOpenTarget,
    markdownFileOpenView,
    htmlFileOpenView,
    imageFileOpenTarget: normalizeMediaFileOpenTarget(source.imageFileOpenTarget),
    videoFileOpenTarget: normalizeMediaFileOpenTarget(source.videoFileOpenTarget),
    audioFileOpenTarget: normalizeMediaFileOpenTarget(source.audioFileOpenTarget),
    /**
     * CDXC:Settings 2026-06-28-08:01:
     * Persist the Show Advanced density switch with other Settings so advanced
     * rows stay visible after a restart until the user disables the switch.
     */
    showAdvancedSettings: readBoolean(source, 'showAdvancedSettings', DEFAULT_ghostex_SETTINGS.showAdvancedSettings),
    /**
     * CDXC:Settings 2026-06-29-17:54:
     * Restart restore reads Settings location from the shared settings file, so
     * normalize active tab and scroll offsets at the storage boundary before
     * React uses them to choose the initial Settings surface.
     */
    settingsModalNavigation: normalizeSettingsModalNavigationState(source.settingsModalNavigation),
    /**
     * CDXC:Settings 2026-06-16-13:08:
     * Normalize the beta gate as a strict boolean so stale or malformed settings
     * cannot expose beta-only OS Integration or other experimental surfaces.
     */
    showBetaFeatures: readBoolean(source, 'showBetaFeatures', DEFAULT_ghostex_SETTINGS.showBetaFeatures),
    codeViewTabHidden: readBoolean(source, 'codeViewTabHidden', DEFAULT_ghostex_SETTINGS.codeViewTabHidden),
    browserViewTabHidden: readBoolean(source, 'browserViewTabHidden', DEFAULT_ghostex_SETTINGS.browserViewTabHidden),
    kanbanViewTabHidden: readBoolean(source, 'kanbanViewTabHidden', DEFAULT_ghostex_SETTINGS.kanbanViewTabHidden),
    automateViewTabHidden: readBoolean(source, 'automateViewTabHidden', DEFAULT_ghostex_SETTINGS.automateViewTabHidden),
    docsViewTabHidden: readBoolean(source, 'docsViewTabHidden', DEFAULT_ghostex_SETTINGS.docsViewTabHidden),
    terminalViewTabHidden: readBoolean(source, 'terminalViewTabHidden', DEFAULT_ghostex_SETTINGS.terminalViewTabHidden),
    ...normalizeProjectWebsiteVisibility(source),
    projectWebsiteViews: normalizeProjectWebsiteSettings(source.projectWebsiteViews),
    storybookViewTabHidden: readBoolean(
      source,
      'storybookViewTabHidden',
      DEFAULT_ghostex_SETTINGS.storybookViewTabHidden
    ),
    botsHidden: readBoolean(source, 'botsHidden', DEFAULT_ghostex_SETTINGS.botsHidden),
    botAutomationsHidden: readBoolean(source, 'botAutomationsHidden', DEFAULT_ghostex_SETTINGS.botAutomationsHidden),
    openDotsHidden: readBoolean(source, 'openDotsHidden', DEFAULT_ghostex_SETTINGS.openDotsHidden),
    tipsAndTricksTitlebarButtonHidden: readBoolean(
      source,
      'tipsAndTricksTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.tipsAndTricksTitlebarButtonHidden
    ),
    notificationsTitlebarButtonHidden: readBoolean(
      source,
      'notificationsTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.notificationsTitlebarButtonHidden
    ),
    helpTitlebarButtonHidden: readBoolean(
      source,
      'helpTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.helpTitlebarButtonHidden
    ),
    resourcesTitlebarButtonHidden: readBoolean(
      source,
      'resourcesTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.resourcesTitlebarButtonHidden
    ),
    devServersTitlebarButtonHidden: readBoolean(
      source,
      'devServersTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.devServersTitlebarButtonHidden
    ),
    extensionsTitlebarButtonHidden: readBoolean(
      source,
      'extensionsTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.extensionsTitlebarButtonHidden
    ),
    gitActionsTitlebarButtonHidden: readBoolean(
      source,
      'gitActionsTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.gitActionsTitlebarButtonHidden
    ),
    quickActionsTitlebarButtonHidden: readBoolean(
      source,
      'quickActionsTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.quickActionsTitlebarButtonHidden
    ),
    openInTitlebarButtonHidden: readBoolean(
      source,
      'openInTitlebarButtonHidden',
      DEFAULT_ghostex_SETTINGS.openInTitlebarButtonHidden
    ),
    /**
     * CDXC:CodeEditor 2026-06-08-20:12:
     * Normalize the code-server VS Code settings-link toggles on every read so
     * missing values use the bundled editor defaults while explicit local VS
     * Code settings choices remain persisted.
     */
    codeServerLinkVscodeUserConfig: readBoolean(
      source,
      'codeServerLinkVscodeUserConfig',
      DEFAULT_ghostex_SETTINGS.codeServerLinkVscodeUserConfig
    ),
    codeServerUseVscodeInsidersUserConfig: readBoolean(
      source,
      'codeServerUseVscodeInsidersUserConfig',
      DEFAULT_ghostex_SETTINGS.codeServerUseVscodeInsidersUserConfig
    ),
    defaultEditorCommand: normalizeDefaultEditorCommand(
      readString(source, 'defaultEditorCommand', DEFAULT_ghostex_SETTINGS.defaultEditorCommand)
    ),
    customDefaultEditorCommand: normalizeCustomDefaultEditorCommand(
      readString(source, 'customDefaultEditorCommand', DEFAULT_ghostex_SETTINGS.customDefaultEditorCommand)
    ),
    // CDXC:Icons 2026-06-25-21:50: Coerce stored app icon source id to a trimmed string, defaulting to the bundled icon.
    appIconSourceId: normalizeAppIconSourceId(
      readString(source, 'appIconSourceId', DEFAULT_ghostex_SETTINGS.appIconSourceId)
    ),
    /**
     * CDXC:Git 2026-05-16-08:46:
     * Missing project-header visibility now follows the Codex preset, which
     * hides git line deltas unless the user selects Detailed or changes this
     * setting directly.
     */
    hideProjectHeaderDiffStats: readBoolean(
      source,
      'hideProjectHeaderDiffStats',
      DEFAULT_ghostex_SETTINGS.hideProjectHeaderDiffStats
    ),
    manageAdditionalDocsFolders: normalizeManageAdditionalDocsFolders(
      readString(source, 'manageAdditionalDocsFolders', DEFAULT_ghostex_SETTINGS.manageAdditionalDocsFolders)
    ),
    // CDXC:Projects 2026-08-02: Global Defaults for the Projects page fields.
    globalWorktreeCommand: normalizeGlobalWorktreeCommand(
      readString(source, 'globalWorktreeCommand', DEFAULT_ghostex_SETTINGS.globalWorktreeCommand)
    ),
    globalBeadsDisplayKey: normalizeGlobalBeadsDisplayKey(
      readString(source, 'globalBeadsDisplayKey', DEFAULT_ghostex_SETTINGS.globalBeadsDisplayKey)
    ),
    globalBeadsDirectory: normalizeGlobalBeadsDirectory(
      readString(source, 'globalBeadsDirectory', DEFAULT_ghostex_SETTINGS.globalBeadsDirectory)
    ),
    globalDocsDirectory: normalizeGlobalDocsDirectory(
      readString(source, 'globalDocsDirectory', DEFAULT_ghostex_SETTINGS.globalDocsDirectory)
    ),
    /**
     * CDXC:Git 2026-05-15-14:33:
     * Missing or invalid older settings must keep project-header git stats in
     * the quieter default that hides the changed-file count.
     */
    showProjectEditorDiffFileCount: readBoolean(
      source,
      'showProjectEditorDiffFileCount',
      DEFAULT_ghostex_SETTINGS.showProjectEditorDiffFileCount
    ),
    showUntrackedProjectDiffWhenNoTrackedChanges: readBoolean(
      source,
      'showUntrackedProjectDiffWhenNoTrackedChanges',
      DEFAULT_ghostex_SETTINGS.showUntrackedProjectDiffWhenNoTrackedChanges
    ),
    completionSound: normalizeCompletionSoundPreference(source),
    copySound: readBoolean(source, 'copySound', DEFAULT_ghostex_SETTINGS.copySound),
    showNotificationOnTerminalBell: readBoolean(
      source,
      'showNotificationOnTerminalBell',
      DEFAULT_ghostex_SETTINGS.showNotificationOnTerminalBell
    ),
    createSessionOnSidebarDoubleClick: readBoolean(
      source,
      'createSessionOnSidebarDoubleClick',
      DEFAULT_ghostex_SETTINGS.createSessionOnSidebarDoubleClick
    ),
    sidebarSessionCycleSkipsSleeping: readBoolean(
      source,
      'sidebarSessionCycleSkipsSleeping',
      DEFAULT_ghostex_SETTINGS.sidebarSessionCycleSkipsSleeping
    ),
    enableSessionParking: readBoolean(source, 'enableSessionParking', DEFAULT_ghostex_SETTINGS.enableSessionParking),
    sleepSessionWhenParking: readBoolean(
      source,
      'sleepSessionWhenParking',
      DEFAULT_ghostex_SETTINGS.sleepSessionWhenParking
    ),
    showTagMenuWhenParking: readBoolean(
      source,
      'showTagMenuWhenParking',
      DEFAULT_ghostex_SETTINGS.showTagMenuWhenParking
    ),
    unparkAfterSendingMessage: readBoolean(
      source,
      'unparkAfterSendingMessage',
      DEFAULT_ghostex_SETTINGS.unparkAfterSendingMessage
    ),
    analyticsEnabled: readBoolean(source, 'analyticsEnabled', DEFAULT_ghostex_SETTINGS.analyticsEnabled),
    debuggingMode: readBoolean(source, 'debuggingMode', DEFAULT_ghostex_SETTINGS.debuggingMode),
    diagnosticLogging: normalizeDiagnosticLoggingSettings(source.diagnosticLogging),
    renameSessionOnDoubleClick: readBoolean(
      source,
      'renameSessionOnDoubleClick',
      DEFAULT_ghostex_SETTINGS.renameSessionOnDoubleClick
    ),
    showProjectIcons: readBoolean(source, 'showProjectIcons', DEFAULT_ghostex_SETTINGS.showProjectIcons),
    /**
     * CDXC:Sessions 2026-05-16-08:46:
     * Missing session-card icon visibility now follows the Codex preset, which
     * hides agent icons until hover unless the user selects Detailed or changes
     * this setting directly.
     */
    hideSessionAgentIconUntilHover: readBoolean(
      source,
      'hideSessionAgentIconUntilHover',
      DEFAULT_ghostex_SETTINGS.hideSessionAgentIconUntilHover
    ),
    /**
     * CDXC:Browser 2026-05-28-07:38:
     * Missing browser-favicon visibility should follow the sidebar preset
     * independently from the older agent-icon hover-only setting so browser
     * page identity does not disappear just because agent logos are quiet.
     */
    hideBrowserFaviconUntilHover: readBoolean(
      source,
      'hideBrowserFaviconUntilHover',
      DEFAULT_ghostex_SETTINGS.hideBrowserFaviconUntilHover
    ),
    sessionCardHoverButtons: normalizeSessionCardHoverButtonsSetting(source),
    showSessionCardHoverButtonsInContextMenu: readBoolean(
      source,
      'showSessionCardHoverButtonsInContextMenu',
      DEFAULT_ghostex_SETTINGS.showSessionCardHoverButtonsInContextMenu
    ),
    hideAccountEmails: readBoolean(source, 'hideAccountEmails', DEFAULT_ghostex_SETTINGS.hideAccountEmails),
    /**
     * CDXC:Sessions 2026-05-15-08:57
     * Older settings files should preserve the current session-card timestamp
     * behavior. Explicit true hides only the Last Active label, not the code
     * project header's separate git additions/deletions summary.
     */
    hideLastActiveTimeOnSessionCards: readBoolean(
      source,
      'hideLastActiveTimeOnSessionCards',
      DEFAULT_ghostex_SETTINGS.hideLastActiveTimeOnSessionCards
    ),
    highlightPendingQuestions: readBoolean(
      source,
      'highlightPendingQuestions',
      DEFAULT_ghostex_SETTINGS.highlightPendingQuestions
    ),
    sidebarSessionTagListItems: normalizeSidebarSessionTagListItems(source.sidebarSessionTagListItems),
    /**
     * CDXC:SessionSleep 2026-05-28-08:06:
     * Normalize Auto Sleep policy independently from keep-awake so Mac power
     * assertions and Ghostex session retirement can be configured separately.
     */
    autoSleepAgentIdleMinutes: normalizeAutoSleepIdleMinutes(
      source,
      'autoSleepAgentIdleMinutes',
      'autoSleepAgentSessionsEnabled',
      DEFAULT_ghostex_SETTINGS.autoSleepAgentIdleMinutes,
      15
    ),
    autoSleepBrowserIdleMinutes: normalizeAutoSleepIdleMinutes(
      source,
      'autoSleepBrowserIdleMinutes',
      'autoSleepBrowserSessionsEnabled',
      DEFAULT_ghostex_SETTINGS.autoSleepBrowserIdleMinutes,
      10
    ),
    autoSleepCodeEditorIdleMinutes: normalizeAutoSleepIdleMinutes(
      source,
      'autoSleepCodeEditorIdleMinutes',
      'autoSleepCodeEditorEnabled',
      DEFAULT_ghostex_SETTINGS.autoSleepCodeEditorIdleMinutes,
      10
    ),
    autoSleepGitEditorIdleMinutes: normalizeAutoSleepIdleMinutes(
      source,
      'autoSleepGitEditorIdleMinutes',
      'autoSleepGitEditorEnabled',
      DEFAULT_ghostex_SETTINGS.autoSleepGitEditorIdleMinutes,
      5
    ),
    autoSleepProjectEditorIdleMinutes: normalizeAutoSleepIdleMinutes(
      source,
      'autoSleepProjectEditorIdleMinutes',
      'autoSleepProjectEditorEnabled',
      DEFAULT_ghostex_SETTINGS.autoSleepProjectEditorIdleMinutes,
      5
    ),
    autoSleepRequireAgentResumeCommand: readBoolean(
      source,
      'autoSleepRequireAgentResumeCommand',
      DEFAULT_ghostex_SETTINGS.autoSleepRequireAgentResumeCommand
    ),
    autoSleepFavoriteAgentSessions: readBoolean(
      source,
      'autoSleepFavoriteAgentSessions',
      DEFAULT_ghostex_SETTINGS.autoSleepFavoriteAgentSessions
    ),
    keepAwakeActivateOnExternalDisplay: readBoolean(
      source,
      'keepAwakeActivateOnExternalDisplay',
      DEFAULT_ghostex_SETTINGS.keepAwakeActivateOnExternalDisplay
    ),
    keepAwakeActivateOnLaunch: readBoolean(
      source,
      'keepAwakeActivateOnLaunch',
      DEFAULT_ghostex_SETTINGS.keepAwakeActivateOnLaunch
    ),
    keepAwakeAllowDisplaySleep: readBoolean(
      source,
      'keepAwakeAllowDisplaySleep',
      DEFAULT_ghostex_SETTINGS.keepAwakeAllowDisplaySleep
    ),
    keepAwakeBatteryThresholdPercent: normalizeKeepAwakeBatteryThresholdPercent(source),
    keepAwakeDeactivateOnLowPowerMode: readBoolean(
      source,
      'keepAwakeDeactivateOnLowPowerMode',
      DEFAULT_ghostex_SETTINGS.keepAwakeDeactivateOnLowPowerMode
    ),
    keepAwakeDeactivateOnUserSwitch: readBoolean(
      source,
      'keepAwakeDeactivateOnUserSwitch',
      DEFAULT_ghostex_SETTINGS.keepAwakeDeactivateOnUserSwitch
    ),
    keepAwakeDefaultDurationMinutes: normalizeKeepAwakeDurationMinutes(
      readNumber(source, 'keepAwakeDefaultDurationMinutes', DEFAULT_ghostex_SETTINGS.keepAwakeDefaultDurationMinutes)
    ),
    keepAwakeWhileWorkingSessions: readBoolean(
      source,
      'keepAwakeWhileWorkingSessions',
      DEFAULT_ghostex_SETTINGS.keepAwakeWhileWorkingSessions
    ),
    keepAwakePreventLidSleep: readBoolean(
      source,
      'keepAwakePreventLidSleep',
      DEFAULT_ghostex_SETTINGS.keepAwakePreventLidSleep
    ),
    /**
     * CDXC:KeepAwake 2026-05-27-07:32:
     * Normalize the hide preference independently from the caffeinate rules so
     * hiding titlebar chrome does not rewrite existing power automation settings.
     *
     * CDXC:KeepAwake 2026-06-19-13:13:
     * Keep the persisted hide preference independent from the beta gate because
     * the titlebar bridge computes effective visibility from both settings.
     */
    hideKeepAwakeTitlebarControl: readBoolean(
      source,
      'hideKeepAwakeTitlebarControl',
      DEFAULT_ghostex_SETTINGS.hideKeepAwakeTitlebarControl
    ),
    hideTabStripNewTerminalButton: readBoolean(
      source,
      'hideTabStripNewTerminalButton',
      DEFAULT_ghostex_SETTINGS.hideTabStripNewTerminalButton
    ),
    hideTabStripNewBrowserButton: readBoolean(
      source,
      'hideTabStripNewBrowserButton',
      DEFAULT_ghostex_SETTINGS.hideTabStripNewBrowserButton
    ),
    /**
     * CDXC:Notifications 2026-05-10-16:46
     * Older settings files should opt into macOS attention notifications, and
     * explicit false must be preserved for users who disable system banners.
     */
    showMacOSAttentionNotifications: readBoolean(
      source,
      'showMacOSAttentionNotifications',
      DEFAULT_ghostex_SETTINGS.showMacOSAttentionNotifications
    ),
    hideMenuBarSessionStatusIndicators: readBoolean(
      source,
      'hideMenuBarSessionStatusIndicators',
      DEFAULT_ghostex_SETTINGS.hideMenuBarSessionStatusIndicators
    ),
    petOverlayEnabled: readBoolean(source, 'petOverlayEnabled', DEFAULT_ghostex_SETTINGS.petOverlayEnabled),
    selectedPetId: normalizePetId(readString(source, 'selectedPetId', DEFAULT_ghostex_SETTINGS.selectedPetId)),
    showQuickModelPickerInTerminal: readBoolean(
      source,
      'showQuickModelPickerInTerminal',
      DEFAULT_ghostex_SETTINGS.showQuickModelPickerInTerminal
    ),
    /**
     * CDXC:Workarea 2026-05-23-00:50:
     * Older settings should normalize the session-id overlay preference from
     * the canonical default while preserving explicit user choices.
     * The native pane still suppresses the actual label unless that terminal is
     * backed by zmx, tmux, or zellij.
     */
    showSessionIdInTerminalPanes: readBoolean(
      source,
      'showSessionIdInTerminalPanes',
      DEFAULT_ghostex_SETTINGS.showSessionIdInTerminalPanes
    ),
    preferredAgentInterface: normalizePreferredAgentInterface(
      readString(source, 'preferredAgentInterface', DEFAULT_ghostex_SETTINGS.preferredAgentInterface)
    ),
    preferredAgentInterfaceOverrides: normalizePreferredAgentInterfaceOverrides(
      source['preferredAgentInterfaceOverrides']
    ),
    sidebarCollapseAnimationDurationMs: clampSidebarCollapseAnimationDurationMs(
      readNumber(
        source,
        'sidebarCollapseAnimationDurationMs',
        DEFAULT_ghostex_SETTINGS.sidebarCollapseAnimationDurationMs
      )
    ),
    panelAnimationSpeed: normalizePanelAnimationSpeed(
      readString(source, 'panelAnimationSpeed', DEFAULT_ghostex_SETTINGS.panelAnimationSpeed)
    ),
    closeSidePanelWithLastTab: readBoolean(
      source,
      'closeSidePanelWithLastTab',
      DEFAULT_ghostex_SETTINGS.closeSidePanelWithLastTab
    ),
    sidebarTooltipDelayMs: clampSidebarTooltipDelayMs(
      readNumber(source, 'sidebarTooltipDelayMs', DEFAULT_ghostex_SETTINGS.sidebarTooltipDelayMs)
    ),
    sidebarDefaultWidthPx: clampSidebarDefaultWidthPx(
      readNumber(source, 'sidebarDefaultWidthPx', DEFAULT_ghostex_SETTINGS.sidebarDefaultWidthPx)
    ),
    projectSessionListCollapsedCount: clampProjectSessionListCollapsedCount(
      readNumber(source, 'projectSessionListCollapsedCount', DEFAULT_ghostex_SETTINGS.projectSessionListCollapsedCount)
    ),
    sidebarSpacesEnabled: readBoolean(source, 'sidebarSpacesEnabled', DEFAULT_ghostex_SETTINGS.sidebarSpacesEnabled),
    sidebarSpaceSwitchBehavior: normalizeSidebarSpaceSwitchBehavior(
      readString(source, 'sidebarSpaceSwitchBehavior', DEFAULT_ghostex_SETTINGS.sidebarSpaceSwitchBehavior)
    ),
    projectSwitchKeepAliveMinutes: clampProjectSwitchKeepAliveMinutes(
      readNumber(source, 'projectSwitchKeepAliveMinutes', DEFAULT_ghostex_SETTINGS.projectSwitchKeepAliveMinutes)
    ),
    sidebarSpaceFollowActiveSession: readBoolean(
      source,
      'sidebarSpaceFollowActiveSession',
      DEFAULT_ghostex_SETTINGS.sidebarSpaceFollowActiveSession
    ),
    sidebarVisibilityMemory: normalizeSidebarVisibilityMemory(
      readString(source, 'sidebarVisibilityMemory', DEFAULT_ghostex_SETTINGS.sidebarVisibilityMemory)
    ),
    expandCollapsedProjectsOnJump: readBoolean(
      source,
      'expandCollapsedProjectsOnJump',
      DEFAULT_ghostex_SETTINGS.expandCollapsedProjectsOnJump
    ),
    showLessForExpandedProjectJumps: readBoolean(
      source,
      'showLessForExpandedProjectJumps',
      DEFAULT_ghostex_SETTINGS.showLessForExpandedProjectJumps
    ),
    sidebarTheme: clampSidebarThemeSetting(readString(source, 'sidebarTheme', DEFAULT_ghostex_SETTINGS.sidebarTheme)),
    sessionChatTheme: normalizeSessionChatTheme(source.sessionChatTheme),
    sessionChatFontFamily: readString(
      source,
      'sessionChatFontFamily',
      DEFAULT_ghostex_SETTINGS.sessionChatFontFamily
    ).trim(),
    sessionChatZoomPercent: clampSessionChatZoomPercent(
      readNumber(source, 'sessionChatZoomPercent', DEFAULT_ghostex_SETTINGS.sessionChatZoomPercent)
    ),
    sessionChatCustomTranscriptWidthEnabled: readBoolean(
      source,
      'sessionChatCustomTranscriptWidthEnabled',
      DEFAULT_ghostex_SETTINGS.sessionChatCustomTranscriptWidthEnabled
    ),
    sessionChatTranscriptWidthPercent: clampSessionChatTranscriptWidthPercent(
      readNumber(
        source,
        'sessionChatTranscriptWidthPercent',
        DEFAULT_ghostex_SETTINGS.sessionChatTranscriptWidthPercent
      )
    ),
    sessionChatFileEditPreviews: readBoolean(
      source,
      'sessionChatFileEditPreviews',
      DEFAULT_ghostex_SETTINGS.sessionChatFileEditPreviews
    ),
    sessionChatKeepComposerExpanded: readBoolean(
      source,
      'sessionChatKeepComposerExpanded',
      DEFAULT_ghostex_SETTINGS.sessionChatKeepComposerExpanded
    ),
    sessionChatConfirmEscapeInterrupt: readBoolean(
      source,
      'sessionChatConfirmEscapeInterrupt',
      DEFAULT_ghostex_SETTINGS.sessionChatConfirmEscapeInterrupt
    ),
    sessionChatVerboseMode: readBoolean(
      source,
      'sessionChatVerboseMode',
      DEFAULT_ghostex_SETTINGS.sessionChatVerboseMode
    ),
    sessionChatSimpleMode: readBoolean(source, 'sessionChatSimpleMode', DEFAULT_ghostex_SETTINGS.sessionChatSimpleMode),
    customSidebarTitlebarForegroundColor: getSidebarTitlebarForegroundForBackground(
      customSidebarTitlebarBackgroundColor
    ),
    customSidebarTitlebarBackgroundTintColor,
    customSidebarTitlebarBackgroundDarknessPercent,
    customSidebarTitlebarBackgroundColor,
    darkThemePreset,
    lightThemePreset,
    themeSidebarContrast,
    themeWorkAreaContrast,
    customSidebarTitlebarLightBackgroundTintColor,
    customSidebarTitlebarLightBackgroundLightnessPercent,
    customSidebarTitlebarLightBackgroundColor,
    terminalCursorStyle: normalizeTerminalCursorStyle(
      readString(source, 'terminalCursorStyle', DEFAULT_ghostex_SETTINGS.terminalCursorStyle)
    ),
    terminalCursorStyleBlink: readBoolean(
      source,
      'terminalCursorStyleBlink',
      DEFAULT_ghostex_SETTINGS.terminalCursorStyleBlink
    ),
    windowsTerminalBackend:
      source.windowsTerminalBackend === 'wsl' ? 'wsl' : DEFAULT_ghostex_SETTINGS.windowsTerminalBackend,
    windowsWslDistribution: normalizeWindowsWslDistribution(
      readString(source, 'windowsWslDistribution', DEFAULT_ghostex_SETTINGS.windowsWslDistribution)
    ),
    /**
     * CDXC:Terminal 2026-04-29-09:32
     * Font family is a raw Ghostty font-family string so users can type any
     * installed font from `ghostty +list-fonts`. Empty means ghostex leaves an
     * existing Ghostty font-family line or Ghostty's platform default in charge.
     * Legacy preset labels are converted to their Ghostty family name.
     */
    terminalFontFamily: normalizeGhosttyFontFamily(
      readString(source, 'terminalFontFamily', DEFAULT_ghostex_SETTINGS.terminalFontFamily)
    ),
    terminalFontSize: clampNumber(
      readNumber(source, 'terminalFontSize', DEFAULT_ghostex_SETTINGS.terminalFontSize),
      8,
      32,
      DEFAULT_ghostex_SETTINGS.terminalFontSize
    ),
    terminalFontWeight: clampNumber(
      readNumber(source, 'terminalFontWeight', DEFAULT_ghostex_SETTINGS.terminalFontWeight),
      100,
      900,
      DEFAULT_ghostex_SETTINGS.terminalFontWeight
    ),
    /**
     * CDXC:Theming 2026-04-29-09:32
     * Ghostty themes are exact strings, including user-defined themes imported from Ghostty config.
     * An empty unmanaged value keeps an existing user-authored theme outside Ghostex control.
     */
    terminalColorScheme: normalizeContentThemeSetting(source.terminalColorScheme),
    terminalGhosttyLightTheme:
      normalizeGhosttyTheme(
        readString(source, 'terminalGhosttyLightTheme', DEFAULT_ghostex_SETTINGS.terminalGhosttyLightTheme)
      ) || DEFAULT_ghostex_SETTINGS.terminalGhosttyLightTheme,
    terminalGhosttyTheme: normalizeGhosttyTheme(
      readString(source, 'terminalGhosttyTheme', DEFAULT_ghostex_SETTINGS.terminalGhosttyTheme)
    ),
    terminalBackgroundImage: readString(
      source,
      'terminalBackgroundImage',
      DEFAULT_ghostex_SETTINGS.terminalBackgroundImage
    ).trim(),
    terminalBackgroundImageOpacity: clampNumber(
      readNumber(source, 'terminalBackgroundImageOpacity', DEFAULT_ghostex_SETTINGS.terminalBackgroundImageOpacity),
      0,
      1,
      DEFAULT_ghostex_SETTINGS.terminalBackgroundImageOpacity
    ),
    terminalBackgroundImageFit: normalizeTerminalBackgroundImageFit(
      readString(source, 'terminalBackgroundImageFit', DEFAULT_ghostex_SETTINGS.terminalBackgroundImageFit)
    ),
    terminalLetterSpacing: clampNumber(
      readNumber(source, 'terminalLetterSpacing', DEFAULT_ghostex_SETTINGS.terminalLetterSpacing),
      -2,
      8,
      DEFAULT_ghostex_SETTINGS.terminalLetterSpacing
    ),
    terminalLineHeight: clampNumber(
      readNumber(source, 'terminalLineHeight', DEFAULT_ghostex_SETTINGS.terminalLineHeight),
      0.8,
      2,
      DEFAULT_ghostex_SETTINGS.terminalLineHeight
    ),
    terminalViewWidthMode: normalizeTerminalViewWidthMode(source),
    terminalViewWidthPercent: clampTerminalViewWidthPercent(
      readNumber(source, 'terminalViewWidthPercent', DEFAULT_ghostex_SETTINGS.terminalViewWidthPercent)
    ),
    terminalWidthApplyToCommandPaneTerminals: readBoolean(
      source,
      'terminalWidthApplyToCommandPaneTerminals',
      DEFAULT_ghostex_SETTINGS.terminalWidthApplyToCommandPaneTerminals
    ),
    /**
     * CDXC:Terminal 2026-09-28 DECISION:
     * User: "make the padding 0 on the gpui app by default". Missing settings use no terminal inset (this replaces the 16px default that matched Chat's inset); Rust's `DEFAULT_TERMINAL_PANE_HORIZONTAL_PADDING_PX` in apps/desktop/src/shared_settings/defaults.rs must match. Explicit values are integer pixels clamped to the Settings slider range so native layout receives bounded inner padding without adding spacing between adjacent panes.
     */
    terminalPaneHorizontalPaddingPx: clampTerminalPanePaddingPx(
      readNumber(source, 'terminalPaneHorizontalPaddingPx', DEFAULT_ghostex_SETTINGS.terminalPaneHorizontalPaddingPx)
    ),
    terminalPaneVerticalPaddingPx: clampTerminalPanePaddingPx(
      readNumber(source, 'terminalPaneVerticalPaddingPx', DEFAULT_ghostex_SETTINGS.terminalPaneVerticalPaddingPx)
    ),
    /**
     * CDXC:Terminal 2026-04-29-08:56
     * Ghostty exposes mouse wheel speed through mouse-scroll-multiplier with
     * separate precision and discrete device prefixes. Store both values so
     * trackpads and notched mouse wheels can be tuned independently while
     * matching the settings modal's 0.25-step practical range. Ghostty accepts
     * 0.01..10000, but those extremes are intentionally not exposed because
     * the docs warn they produce a bad experience.
     */
    terminalMouseScrollMultiplierDiscrete: clampNumber(
      readNumber(
        source,
        'terminalMouseScrollMultiplierDiscrete',
        DEFAULT_ghostex_SETTINGS.terminalMouseScrollMultiplierDiscrete
      ),
      MIN_GHOSTTY_MOUSE_SCROLL_MULTIPLIER,
      MAX_GHOSTTY_MOUSE_SCROLL_MULTIPLIER,
      DEFAULT_ghostex_SETTINGS.terminalMouseScrollMultiplierDiscrete
    ),
    terminalMouseScrollMultiplierPrecision: clampNumber(
      readNumber(
        source,
        'terminalMouseScrollMultiplierPrecision',
        DEFAULT_ghostex_SETTINGS.terminalMouseScrollMultiplierPrecision
      ),
      MIN_GHOSTTY_MOUSE_SCROLL_MULTIPLIER,
      MAX_GHOSTTY_MOUSE_SCROLL_MULTIPLIER,
      DEFAULT_ghostex_SETTINGS.terminalMouseScrollMultiplierPrecision
    ),
    terminalScrollToBottomWhenTyping: readBoolean(
      source,
      'terminalScrollToBottomWhenTyping',
      DEFAULT_ghostex_SETTINGS.terminalScrollToBottomWhenTyping
    ),
    /**
     * CDXC:Terminal 2026-04-29-09:32
     * Common Ghostty terminal behavior settings are persisted with the same
     * practical UI ranges and enum values that the settings modal exposes,
     * then written as documented Ghostty config keys by the native host.
     */
    terminalScrollbackLimitMb: clampNumber(
      readNumber(source, 'terminalScrollbackLimitMb', DEFAULT_ghostex_SETTINGS.terminalScrollbackLimitMb),
      MIN_GHOSTTY_SCROLLBACK_LIMIT_MB,
      MAX_GHOSTTY_SCROLLBACK_LIMIT_MB,
      DEFAULT_ghostex_SETTINGS.terminalScrollbackLimitMb
    ),
    terminalCopyOnSelect: normalizeGhosttyCopyOnSelect(
      readString(source, 'terminalCopyOnSelect', DEFAULT_ghostex_SETTINGS.terminalCopyOnSelect)
    ),
    terminalConfirmCloseSurface: normalizeGhosttyConfirmCloseSurface(
      readString(source, 'terminalConfirmCloseSurface', DEFAULT_ghostex_SETTINGS.terminalConfirmCloseSurface)
    ),
    /**
     * CDXC:Terminal 2026-04-29-09:32
     * Clipboard cleanup/protection and mouse/scrollbar visibility mirror
     * Ghostty's documented defaults unless the user changes them in ghostex.
     */
    terminalClipboardTrimTrailingSpaces: readBoolean(
      source,
      'terminalClipboardTrimTrailingSpaces',
      DEFAULT_ghostex_SETTINGS.terminalClipboardTrimTrailingSpaces
    ),
    terminalClipboardPasteProtection: readBoolean(
      source,
      'terminalClipboardPasteProtection',
      DEFAULT_ghostex_SETTINGS.terminalClipboardPasteProtection
    ),
    terminalPastePreviewableImages: readBoolean(
      source,
      'terminalPastePreviewableImages',
      DEFAULT_ghostex_SETTINGS.terminalPastePreviewableImages
    ),
    terminalMouseHideWhileTyping: readBoolean(
      source,
      'terminalMouseHideWhileTyping',
      DEFAULT_ghostex_SETTINGS.terminalMouseHideWhileTyping
    ),
    terminalScrollbar: normalizeGhosttyScrollbar(
      readString(source, 'terminalScrollbar', DEFAULT_ghostex_SETTINGS.terminalScrollbar)
    ),
    /**
     * CDXC:Resources 2026-06-23-19:22:
     * Dev-server settings normalize in the app layer because they are not Ghostty keys. Canonicalize ignored port rules to sorted, merged strings.
     *
     * CDXC:Navigation 2026-08-19:
     * The launch choice moved to webLinkOpenTarget, which absorbs both the legacy dev-server target and its older per-browser default.
     */
    terminalDevServerDetectionEnabled: readBoolean(
      source,
      'terminalDevServerDetectionEnabled',
      DEFAULT_ghostex_SETTINGS.terminalDevServerDetectionEnabled
    ),
    terminalDevServerIgnoredPortRules: normalizeTerminalDevServerIgnoredPortRules(
      source.terminalDevServerIgnoredPortRules
    ),
    /**
     * CDXC:Portless 2026-06-22-22:35:
     * Portless normalization accepts only explicit booleans and lowercase http/https. Missing, legacy, string-boolean, and invalid values fall back to enabled HTTPS without preserving project-scoped Portless keys.
     */
    portlessEnabled: readBoolean(source, 'portlessEnabled', DEFAULT_ghostex_SETTINGS.portlessEnabled),
    portlessProtocol: normalizePortlessProtocol(
      readString(source, 'portlessProtocol', DEFAULT_ghostex_SETTINGS.portlessProtocol)
    ),
    promptEditorBackend,
    /**
     * CDXC:Hotkeys 2026-04-28-05:20
     * User-defined app shortcuts are normalized with defaults on every settings
     * read so older settings files gain configurable native hotkeys without a
     * migration or fallback execution path.
     */
    hotkeys: normalizeghostexHotkeySettings(source.hotkeys),
    showActivePaneOutline: readBoolean(source, 'showActivePaneOutline', DEFAULT_ghostex_SETTINGS.showActivePaneOutline),
    windowGlass: normalizeWindowGlassMode(readString(source, 'windowGlass', DEFAULT_ghostex_SETTINGS.windowGlass)),
    windowGlassSource: normalizeWindowGlassSource(
      readString(source, 'windowGlassSource', DEFAULT_ghostex_SETTINGS.windowGlassSource)
    ),
    windowGlassImagePlacement: normalizeWindowGlassImagePlacement(
      readString(source, 'windowGlassImagePlacement', DEFAULT_ghostex_SETTINGS.windowGlassImagePlacement)
    ),
    windowGlassImageDark: readString(
      source,
      'windowGlassImageDark',
      DEFAULT_ghostex_SETTINGS.windowGlassImageDark
    ).trim(),
    windowGlassImageLight: readString(
      source,
      'windowGlassImageLight',
      DEFAULT_ghostex_SETTINGS.windowGlassImageLight
    ).trim(),
    windowGlassVideoDark: normalizeWindowGlassVideoPath(
      readString(source, 'windowGlassVideoDark', DEFAULT_ghostex_SETTINGS.windowGlassVideoDark)
    ),
    windowGlassVideoLight: normalizeWindowGlassVideoPath(
      readString(source, 'windowGlassVideoLight', DEFAULT_ghostex_SETTINGS.windowGlassVideoLight)
    ),
    windowGlassVideoOnlyOnPower: readBoolean(
      source,
      'windowGlassVideoOnlyOnPower',
      DEFAULT_ghostex_SETTINGS.windowGlassVideoOnlyOnPower
    ),
    windowGlassLiveStyleDark: normalizeWindowGlassLiveSlot(source, 'Dark'),
    windowGlassLiveStyleLight: normalizeWindowGlassLiveSlot(source, 'Light'),
    windowGlassLiveSpeed: normalizeWindowGlassLiveSpeed(
      readNumber(source, 'windowGlassLiveSpeed', DEFAULT_ghostex_SETTINGS.windowGlassLiveSpeed)
    ),
    windowGlassLiveBrightness: normalizeWindowGlassLiveBrightness(
      readNumber(source, 'windowGlassLiveBrightness', DEFAULT_ghostex_SETTINGS.windowGlassLiveBrightness)
    ),
    windowGlassBlurRadius: normalizeWindowGlassBlurRadius(
      readNumber(source, 'windowGlassBlurRadius', DEFAULT_ghostex_SETTINGS.windowGlassBlurRadius),
      DEFAULT_ghostex_SETTINGS.windowGlassBlurRadius
    ),
    windowGlassMenuBlurRadius: normalizeWindowGlassBlurRadius(
      readNumber(source, 'windowGlassMenuBlurRadius', DEFAULT_ghostex_SETTINGS.windowGlassMenuBlurRadius),
      DEFAULT_ghostex_SETTINGS.windowGlassMenuBlurRadius
    ),
    windowGlassSidebarOpacityDark: clampWindowGlassSidebarOpacityPercent(
      readNumber(source, 'windowGlassSidebarOpacityDark', DEFAULT_ghostex_SETTINGS.windowGlassSidebarOpacityDark),
      DEFAULT_ghostex_SETTINGS.windowGlassSidebarOpacityDark
    ),
    windowGlassWorkAreaTintDark: normalizeWindowGlassWorkAreaTint(source, 'Dark'),
    windowGlassSidebarOpacityLight: clampWindowGlassSidebarOpacityPercent(
      readNumber(source, 'windowGlassSidebarOpacityLight', DEFAULT_ghostex_SETTINGS.windowGlassSidebarOpacityLight),
      DEFAULT_ghostex_SETTINGS.windowGlassSidebarOpacityLight
    ),
    windowGlassWorkAreaTintLight: normalizeWindowGlassWorkAreaTint(source, 'Light'),
    workspaceActivePaneBorderColor:
      readString(
        source,
        'workspaceActivePaneBorderColor',
        DEFAULT_ghostex_SETTINGS.workspaceActivePaneBorderColor
      ).trim() || DEFAULT_ghostex_SETTINGS.workspaceActivePaneBorderColor,
    /**
     * CDXC:Theming 2026-09-28 DECISION:
     * User: "allow us to set terminal background color to full black/white and make it black by default not the background color from the theme". Terminal background is a choice of Black / white (pure black behind dark terminals, pure white behind light ones; the default), Follow theme (the theme's content colour), or Custom (`workspaceBackgroundColor`, dark mode only). It only colours the terminal panes, never the work area. A settings file saved before this choice existed keeps its custom colour, and one that followed the theme by default moves to the new black default. Supersedes the 2026-09-23 decision that the terminal background follows the theme by default.
     */
    terminalBackgroundMode: normalizeTerminalBackgroundMode(source),
    workspaceBackgroundColor: normalizeTerminalBackgroundSetting(
      readString(source, 'workspaceBackgroundColor', DEFAULT_ghostex_SETTINGS.workspaceBackgroundColor)
    ),
    clickToWakeSleepingSessions: readBoolean(
      source,
      'clickToWakeSleepingSessions',
      DEFAULT_ghostex_SETTINGS.clickToWakeSleepingSessions
    ),
    dimSleepingSessions: readBoolean(source, 'dimSleepingSessions', DEFAULT_ghostex_SETTINGS.dimSleepingSessions),
    wakeSleepingSessionsOnSelect: readBoolean(
      source,
      'wakeSleepingSessionsOnSelect',
      DEFAULT_ghostex_SETTINGS.wakeSleepingSessionsOnSelect
    ),
    customViews: normalizeGhostexCustomViews(source.customViews),
    customViewTemplates: normalizeProjectViewTemplates(source.customViewTemplates),
    viewScopes: normalizeGhostexViewScopes(source.viewScopes),
    titlebarViewOrder: normalizeTitlebarViewOrder(source.titlebarViewOrder),
    /**
     * CDXC:Titlebar 2026-05-11-00:22
     * Settings owns which titlebar Open In targets are shown. Normalize on read
     * so the React titlebar can trust the persisted custom commands and hidden
     * built-in ids sent through native layout sync.
     */
    customWorkspaceOpenTargets: normalizeCustomWorkspaceOpenTargets(source.customWorkspaceOpenTargets),
    workspaceOpenTargetAvailability: normalizeWorkspaceOpenTargetAvailability(source.workspaceOpenTargetAvailability),
    workspaceOpenTargetHiddenIds: normalizeWorkspaceOpenTargetHiddenIds(source.workspaceOpenTargetHiddenIds),
    workspacePaneGap: 0,
    remoteMachines: normalizeRemoteMachineSettings(source.remoteMachines),
    remoteTailscaleEnabled: readBoolean(
      source,
      'remoteTailscaleEnabled',
      DEFAULT_ghostex_SETTINGS.remoteTailscaleEnabled
    ),
    commandsPanelDefaultHeightPx: clampCommandsPanelDefaultHeightPx(
      readNumber(source, 'commandsPanelDefaultHeightPx', DEFAULT_ghostex_SETTINGS.commandsPanelDefaultHeightPx)
    ),
    commandsPanelAutoMinimize: readBoolean(
      source,
      'commandsPanelAutoMinimize',
      DEFAULT_ghostex_SETTINGS.commandsPanelAutoMinimize
    ),
    commandsPanelAutoMinimizeDelaySeconds:
      COMMANDS_PANEL_AUTO_MINIMIZE_DELAY_OPTIONS.find(
        (option) => option.value === source.commandsPanelAutoMinimizeDelaySeconds
      )?.value ?? DEFAULT_ghostex_SETTINGS.commandsPanelAutoMinimizeDelaySeconds,
    commandsPanelSide: normalizeCommandsPanelSide(
      readString(source, 'commandsPanelSide', DEFAULT_ghostex_SETTINGS.commandsPanelSide)
    ),
  };
}

export function getTerminalFontFamilyForghostexSettings(settings: ghostexSettings): string {
  return settings.terminalFontFamily.trim() || getTerminalFontFamilyForPreset('JetBrains Mono');
}

export function applySidebarSettingsPreset(
  settings: ghostexSettings,
  presetId: SidebarSettingsPresetId
): ghostexSettings {
  return normalizeghostexSettings({
    ...settings,
    ...SIDEBAR_SETTINGS_PRESET_SETTINGS[presetId],
  });
}
