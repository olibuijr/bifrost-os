#!/usr/bin/env python3
"""Triage the OS layer daemon rows (docs/parity-matrix/os/daemons/*.json) that still carry the blanket heuristic.

Usage:
  os_daemons.py            dry run: print the counts a write would produce
  os_daemons.py --write    rewrite the shards

Ordered rules, first match wins, on the launchd label without its "com.apple." prefix (case-insensitive regex search).
A rule gives a status:
  partial  BifrOSt has a counterpart: repo paths that must exist (a rule whose path is missing is skipped), or a Linux
           base service named in the note
  n/a      an Apple cloud, store, hardware or enterprise service BifrOSt does not mirror, with the reason in the note
Rows no rule matches keep their heuristic "os" status. A row a person assessed (no "heuristic" method) is left alone.
Rows set here carry method "rule" so a rerun reproduces them. Parity still needs a Midget check.
"""
import glob
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SHARDS = os.path.join(REPO, "docs", "parity-matrix", "os", "daemons")
S = "springboard/"

# (regex, status, [repo paths] or None, note)
RULES = [
    # Apple cloud, accounts, store, telemetry and advertising: no counterpart is wanted
    (r"^(cloudd|cloudphotod|cloudpaird|cloudtelemetryd|CloudSettingsSyncAgent|icloud|bird$|itunescloudd|ckdiscretionaryd|cksharingmanagementd|protectedcloudstorage|privatecloudcomputed|SecureBackupDaemon|messagesescrowd|keychainsharingmessagingd|security\.cloudkeychainproxy|ak\.|akd$|appleaccount|appleidsetupd|AOS|apsd|adid|ap\.|ams|amp\.|appstore|appleseed|betaenrollment|commerce|fairplay|financed|news|sports|watchlist|videosubscriptions|storeaccountd|storeassetd|storedownloadd|storekit|storelegacy|storereceiptinstaller|storeuid|analytics|audioanalyticsd|geoanalyticsd|rtcreportingd|ecosystem|cloudtelemetry|feedbackd|dprivacyd|diagnosticspushd|osanalytics|SubmitDiagInfo|UsageTracking|webprivacyd|frauddefensed|transparency|swtransparency|accountsd$)", "n/a", None,
     "Apple cloud, account, store, advertising or telemetry service; BifrOSt has no Apple cloud and mirrors none of it"),
    # enterprise device management
    (r"^(ManagedClient|mdmclient|devicemanagementclient|deviceconfigurationd|DeviceConfigurationAgent|RemoteManagementAgent|remotemanagementd|ManagedSettingsAgent|managedappdistribution|ConfigurationProfiles|eligibilityd|mobile\.obliteration|erasecontentsettingshelperd|EscrowSecurityAlert)", "n/a", None,
     "enterprise management, enrolment or erase service; BifrOSt is not an MDM-managed system"),
    # hardware, firmware and vendor internals
    (r"^(appleh\d+camerad|cameraispd|cameraispdarwinsimd|aned|aneuserd|nand|applessdstatistics|iomfb|IOAccelMemoryInfoCollector|ioupsd|retimerd|uarp|liquiddetectiond|bridgeOSUpdateProxy|xartstorageremoted|touchbarserver|SpacesTouchBarAgent|NowPlayingTouchUI|sidecar|tvremoted|lightsoutmanagementd|mobilerepaird|corerepaird|cvmsComp|cvmsServ|eoshostd|Virtualization\.AppleVirtualPlatformHIDBridge|SafeEjectGPU|DumpGPURestart|ReportGPURestart|gputoolsserviced|MTLAssetUpgraderD|kernelmanager|kdumpd|NVMeAgent|MobileAccessoryUpdater|accessoryupdaterd|mobile\.NRDUpdated|nfcd|seserviced|usbctelemetryd|WirelessRadioManager|CommCenter|jetsamproperties|dumppanic|IFCStart|iomfb|coreduetd|appleKeyStoreD|applekeystored|mobile\.keybagd|mobileactivationd|sensingpredictd|attentionawarenessd|biomed|BiomeAgent|biomesyncd|powerexperienced|powerdatad|perfpower|memory-maintenance|peakpowermanagerd)", "n/a", None,
     "Apple silicon, firmware, secure element or device-specific service; BifrOSt targets generic PC hardware"),
    # Apple Intelligence and on-device assistant plumbing
    (r"^(intelligence|generative|ciphermld|mlhostd|modelmanagerd|modelcatalog|ModelCatalogAgent|GenerativeFunctions|imageplaygroundd|writingtools|visualintelligenced|knowledge|assistant|siri|corespeechd|proactive|suggestd|duetexpertd|ospredictiond|ContextStoreAgent|contextstored|intelligentroutingd|textcomposerd|mediaanalysisd|photoanalysisd|callintelligenced|dasd|triald|seld|griddatad|coreidvd|ModelCatalogAgent|Archetype)", "n/a", None,
     "Apple Intelligence, personalisation or prediction service; the assistant app has its own cloud and local model paths"),
    # counterparts in the repo
    (r"^(mDNSResponder|srp-mdns-proxy)", "partial", [S+"core/src/dnssd.rs", S+"core/src/dns_browse.rs"], "service discovery and DNS-SD model"),
    (r"^(notifyd|distnoted)", "partial", [S+"core/src/notify_center.rs", S+"sdk/src/notify_bus.rs"], "notification centre and notify bus models"),
    (r"^(locationd|CoreLocationAgent|locationaccessstored|countryd|routined|navd|maps\.destinationd)", "partial", [S+"sdk/src/core_location.rs"], "location model; the shell has no location service yet"),
    (r"^(bluetooth|BTServer|BlueTool|btsa|BTServerAgent|bluetoothd|bluetoothuserd|bluetoothUIServer|OBEXAgent)", "partial", [S+"sdk/src/bluetooth_service.rs", S+"core/src/bt_service_browser.rs", S+"core/src/obex_packet.rs"], "Bluetooth service, browser and OBEX models over the Linux stack"),
    (r"^(diskarbitration|diskimages|diskmanagement|diskspaced|unmountassistant|hdiejectd|autofsd|automountd|storagekitd|filesystems\.|fskit|vsdbutil|corestorage|DiskArbitrationAgent|StorageManagement|storage\.)", "partial", [S+"core/src/disk_session.rs", S+"sdk/src/disk_arbitration.rs"], "disk arbitration, image and volume models; Disk Utility app"),
    (r"^(printtool|printuitool|org\.cups|cups)", "partial", [S+"ui/src/cups.rs", S+"ui/src/ipp.rs"], "CUPS client and IPP models; Print Center app"),
    (r"^(quicklook)", "partial", [S+"sdk/src/quick_look.rs", S+"sdk/src/thumbnails.rs"], "Quick Look and thumbnail service models"),
    (r"^(tccd|AskPermissionUI|askpermissiond|accessoryaccess|FeatureAccessAgent)", "partial", [S+"sdk/src/privacy_permissions.rs", S+"core/src/permission_request.rs"], "privacy permission service model"),
    (r"^(Kerberos|gssd|GSSCred|configureLocalKDC|AppSSO|PlatformSSO)", "partial", [S+"sdk/src/gss_krb5.rs"], "GSS and Kerberos models"),
    (r"^(metadata\.|mds|mdworker|DataDetectors|corespotlight)", "partial", [S+"ui/src/searchindex.rs", S+"sdk/src/search_index_batch.rs"], "search index model; Spotlight search in the shell"),
    (r"^(audio\.|audiomxd|usbaudiod|historicalaudiod|coreaudiod|bluetoothaudiod|mediaremote|midiserver)", "partial", [S+"ui/src/audio_engine.rs", S+"sdk/src/audio_devices.rs", S+"audiod"], "audio engine and device models; the audiod service"),
    (r"^(powerd|powerchime|PowerUIAgent|appsleepd|usbpowerd|thermalmonitord|watchdogd|sleep)", "partial", [S+"sdk/src/power_service.rs", S+"core/src/thermal.rs"], "power and thermal service models"),
    (r"^(timed|timezone|tzd|tzlinkd|sntpd|clocksyncd|mobiletimerd|preferences\.timezone)", "partial", [S+"sdk/src/time_zone.rs"], "time zone model; time sync is the Linux timesyncd unit"),
    (r"^(fseventsd|FileCoordination|FileProvider|fpassetmanagerd|filederivatived|revisiond|scopedbookmark)", "partial", [S+"sdk/src/fswatch.rs", S+"sdk/src/file_coordination.rs", S+"sdk/src/file_provider.rs"], "file watching, coordination and provider models"),
    (r"^(logd|syslogd|aslmanager|newsyslog|diagnosticd|diagnostics|signpost|enhancedloggingd|tracd|systemstats|sysmond|perfpowermetricd|metrickitd)", "partial", [S+"core/src/log.rs", S+"core/src/signpost.rs"], "unified logging and signpost models; Console app"),
    (r"^(ReportCrash|CrashReporterSupportHelper|ReportMemoryException|ReportSystemMemory|spindump|tailspind|sysdiagnose|coresymbolicationd|osanalytics)", "partial", [S+"sdk/src/crash_report.rs", S+"sdk/src/call_stack_tree.rs"], "crash report and call-stack models"),
    (r"^(usernoted|chronod|liveactivitiesd|donotdisturbd|FollowUp|followupd)", "partial", [S+"sdk/src/notification.rs", S+"sdk/src/usernotif.rs", S+"ui/src/live_activity.rs"], "notification, live activity and focus models"),
    (r"^(airportd|wifi|WiFiAgent|wifip2pd|captiveagent|eapolcfg|networklink|NetworkLinkConditioner)", "partial", [S+"sdk/src/wlan.rs", S+"sdk/src/wifi_status.rs"], "Wi-Fi models over the Linux wireless stack"),
    (r"^(securityd|secd|secinitd|security\.|trustd|TrustEvaluation|ocspd|systemkeychain|kcproxy|kcgend|taskgated|sandboxd|CoreAuthentication|LocalAuthentication|biometrickit|lockd|lockoutagent|CryptoTokenKit|ctkd|ctkbind|usbsmartcardreaderd|ifdreader|AppleCredentialManager|PasswordService|xprotect|syspolicy|endpointsecurity|CoreAuth)", "partial", [S+"sdk/src/keychain.rs", S+"sdk/src/trust.rs", S+"sdk/src/local_auth.rs"], "keychain, trust and local authentication models"),
    (r"^(calaccessd|calendar|CalendarAgent|remindd|dataaccess|exchange\.exchangesyncd)", "partial", [S+"sdk/src/calendar.rs", S+"sdk/src/dav_sync.rs"], "calendar and sync models; Calendar and Reminders apps"),
    (r"^(contactsd|contacts\.|AddressBook|peopled|peopleservice)", "partial", [S+"sdk/src/contacts.rs"], "contacts model; Contacts app"),
    (r"^(sharingd|rapportd|nearbyd|companiond|relatived|mediacontinuityd|airplay|AirPlay|RapportUIAgent|ensemble|UniversalControl|identityservicesd|idsremote)", "partial", [S+"sdk/src/nearby_service.rs", S+"sdk/src/peer_session.rs"], "nearby and peer session models"),
    (r"^(hidd|IOUIAgent|KernelEventAgent|IOUI|gamepolicyd|gamed)", "partial", [S+"sdk/src/hid_devices.rs", S+"sdk/src/gamepad_service.rs"], "HID and game controller models"),
    (r"^(accessoryd|accessorysensormgrd|accessoryaccessd|accessory\.|AccessoryUpdater|usbmuxd)", "partial", [S+"core/src/accessory_manager.rs", S+"sdk/src/external_accessory.rs"], "accessory manager and external accessory models"),
    (r"^(cfprefsd|syncdefaultsd|systemsettingsagent|UpdateSettings|UserEventAgent|settings)", "partial", [S+"sdk/src/defaults.rs", S+"sdk/src/pref_domains.rs"], "defaults and preference domain models; Settings app"),
    (r"^(coreservices|lsd$|lsd|coreservicesd|swcd|useractivityd|appleevents|eppc|uikitsystemapp|DesktopServices|iconservices|ServicesUIAgent|talagent)", "partial", [S+"sdk/src/launch_services.rs", S+"sdk/src/app_links.rs"], "launch services and app link models"),
    (r"^(pboard|pbs)", "partial", [S+"sdk/src/pasteboard.rs"], "pasteboard model and the shell pasteboard service"),
    (r"^(fontd|FontWorker|fontmover|FontValidator|applespell|handwritingd|textunderstanding)", "partial", [S+"sdk/src/font_descriptor.rs", S+"sdk/src/spellcheck.rs"], "font and spelling models; Font Book app"),
    (r"^(cmio|cameracaptured|corecaptured|avconferenced|avbdeviced|ptpcamerad|icdd|ContinuityCapture|VDCAssistant)", "partial", [S+"ui/src/camera_device.rs", S+"ui/src/device_browser.rs"], "camera and capture device models"),
    (r"^(colorsync|corebrightnessd|displaypolicyd|preference\.displays|OSDUIHelper|AquaAppearanceHelper)", "partial", [S+"render/src/icc.rs", S+"sdk/src/displays.rs"], "colour profile and display models"),
    (r"^(speech\.|speechmaintenanced|speech)", "partial", [S+"sdk/src/speech_recognizer.rs", S+"ui/src/speech.rs"], "speech recognition and synthesis models"),
    (r"^(translationd|naturallanguaged)", "partial", [S+"sdk/src/translation.rs"], "translation session model"),
    (r"^(weatherd)", "partial", [S+"apps/weather-desktop"], "Weather app"),
    (r"^(Maps\.|maps)", "partial", [S+"apps/maps-desktop"], "Maps app"),
    (r"^(voicememod)", "partial", [S+"apps/voicememos-desktop"], "Voice Memos app"),
    (r"^(backgroundassets|backgroundtaskmanagement|nsurlsessiond|jetpackassetd|assetsubscriptiond|mobileassetd|languageassetd|uarpassetmanagerd|assetcache)", "partial", [S+"sdk/src/url_background.rs", S+"ui/src/assetpack.rs"], "background transfer and asset pack models"),
    (r"^(cfnetwork|networkserviceproxy|nehelper|neagent|nesessionmanager|symptomsd|netauth|alf|pfd|pfctl|racoon|ftp-proxy|dhcp6d|bootps|pcapd|NetworkSharing|dprivacyd)", "partial", [S+"sdk/src/network_path.rs", S+"core/src/vpn_manager.rs"], "network path and VPN models; firewall and DHCP are Linux units"),
    (r"^(remoted|RemoteDesktop|RFBEventHelper|remotepairtool|replayd)", "partial", [S+"rfb"], "remote framebuffer service; Remote app"),
    (r"^(accessibility|VoiceOver|universalaccess|AXVisual|AccessibilityVisuals|scrod)", "partial", [S+"ui/src/ax.rs", S+"ui/src/braille.rs", S+"ui/src/captions.rs"], "accessibility models"),
    (r"^(backupd|TMHelper|mbbackground|mbfloagent|mbsystemadministration|mbuseragent|mbusertrampoline|backupd-helper)", "partial", [S+"core/src/snapshot_backup.rs", S+"core/src/backup_exclusion.rs"], "snapshot backup model"),
    (r"^(installd|install|system_installd|PackageUIKit|softwareupdated|suhelperd|mobile\.softwareupdated|ValidUpdater|postupgraded|bootinstalld|installcoordination|InstallerProgress|InstallerDiagnostics|startupdiskhelper|appinstalld|uninstalld|progressd)", "partial", [S+"updated"], "the updated service and the app installer"),
    (r"^(homed|homeenergyd|homeeventsd|ThreadCommissionerService|threadradiod)", "partial", [S+"core/src/matter.rs", S+"core/src/matter_interaction.rs"], "Matter commissioning and interaction models"),
    (r"^(imklaunchagent|RegisterPluginIM|imautomaticgenerat|inputmethod)", "partial", [S+"ui/src/input_method.rs"], "input method models"),
    (r"^(pluginkit|sysextd|extensionkit|xpc\.)", "partial", [S+"sdk/src/extension.rs", S+"ui/src/extensions.rs"], "extension models"),
    (r"^(mobile\.storage_mounter|mobile\.usermanagerd|usermanagerhelper|xpc\.roleaccountd|sessionlogoutd|opendirectoryd|odproxyd|sociallayerd)", "partial", [S+"ui/src/directory_node.rs", S+"core/src/console_user.rs"], "directory and session models"),
    (r"^(AutoFillPanel|autofill)", "partial", [S+"sdk/src/autofill.rs"], "autofill model"),
    (r"^(coredatad|datastored|appplaceholdersyncd|bookdatastored|bookassetd)", "partial", [S+"ui/src/data_store.rs", S+"ui/src/record_store.rs"], "record store models"),
    (r"^(Screen|screensaver)", "partial", [S+"sdk/src/screen_saver_defaults.rs"], "screen saver model"),
    # standard Unix services: the Linux base ships the daemon
    (r"^(ssh|com\.openssh|org\.apache\.httpd|org\.net-snmp|org\.openldap|tftp|ntalk|postfix|smbd|smb\.|nfsd|nfsconf|statd|rpcbind|com\.vix\.cron|getty|atrun|uucp|msrpc|lockd$|auditd|syslogd)", "partial", None, "standard Unix service shipped by the Linux base (a systemd unit); not yet surfaced in Settings"),
]


def load():
    for f in sorted(glob.glob(os.path.join(SHARDS, "*.json"))):
        yield f, json.load(open(f))


def main():
    write = "--write" in sys.argv
    compiled = []
    for pat, status, paths, note in RULES:
        if paths and not all(os.path.exists(os.path.join(REPO, p)) for p in paths):
            missing = [p for p in paths if not os.path.exists(os.path.join(REPO, p))]
            print(f"skip rule {pat[:40]!r}: missing {missing}", file=sys.stderr)
            # keep the rule with the paths that exist
            paths = [p for p in paths if os.path.exists(os.path.join(REPO, p))]
            if not paths:
                continue
        compiled.append((re.compile(pat, re.I), status, paths, note))
    counts = {}
    changed = 0
    for f, d in load():
        dirty = False
        for e in d["entries"]:
            if e.get("status") != "os" or e.get("method") not in ("heuristic", "rule"):
                continue
            label = re.sub(r"^com\.apple\.", "", e["name"])
            for rx, status, paths, note in compiled:
                if rx.search(label):
                    e["status"] = status
                    e["bifrost"] = ", ".join(paths) if paths else ""
                    e["note"] = "rule: " + note
                    e["method"] = "rule"
                    counts[status] = counts.get(status, 0) + 1
                    changed += 1
                    dirty = True
                    break
        if dirty and write:
            with open(f, "w") as fh:
                json.dump(d, fh, indent=1, ensure_ascii=False)
                fh.write("\n")
    print(f"{changed} rows changed {counts}" + ("" if write else " (dry run, use --write)"))


if __name__ == "__main__":
    main()
