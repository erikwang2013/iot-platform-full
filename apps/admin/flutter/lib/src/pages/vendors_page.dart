import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../auth_controller.dart';

/// 厂商接入：OAuth 授权（授权码换 token）/ 凭据直填 / 设备导入。
/// 后端没有厂商状态查询接口，页面只展示本地已知配置，不臆造凭据状态。
class VendorsPage extends StatefulWidget {
  const VendorsPage({super.key});

  @override
  State<VendorsPage> createState() => _VendorsPageState();
}

class _VendorsPageState extends State<VendorsPage> {
  /// 后端 adapter_for 的厂商标识：小米是 miot（不是 xiaomi）。
  static const _vendors = [
    (key: 'tuya', name: 'Tuya', kind: 'OAuth'),
    (key: 'miot', name: 'Xiaomi MIoT', kind: 'OAuth'),
    (key: 'huawei', name: 'Huawei', kind: 'AK/SK'),
    (key: 'aws', name: 'AWS IoT', kind: 'AK/SK'),
    (key: 'azure', name: 'Azure IoT', kind: 'AK/SK'),
  ];

  void _toast(String msg) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _authorize(String vendor) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    try {
      final resp = await api
          .post('/api/v1/access/oauth/authorize-url', body: {'vendor': vendor});
      final url = resp is Map ? resp['url'] : null;
      if (url is! String || url.isEmpty) {
        throw ApiListError('empty authorize url');
      }
      await showDialog<void>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: Text(l10n.adminVendorOAuthUrl),
          content: SingleChildScrollView(child: SelectableText(url)),
          actions: [
            TextButton(
                onPressed: () => Navigator.pop(ctx),
                child: Text(l10n.commonOk)),
          ],
        ),
      );
    } catch (e) {
      _toast('$e');
    }
  }

  Future<void> _saveCreds(String vendor) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final clientId = TextEditingController();
    final clientSecret = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('${l10n.adminVendorSaveCreds} · $vendor'),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: clientId,
                decoration: InputDecoration(
                    labelText: l10n.adminVendorClientId,
                    border: const OutlineInputBorder()),
              ),
              const SizedBox(height: 8),
              TextField(
                controller: clientSecret,
                obscureText: true,
                decoration: InputDecoration(
                    labelText: l10n.adminVendorClientSecret,
                    border: const OutlineInputBorder()),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: Text(l10n.commonCancel)),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: Text(l10n.commonSave)),
        ],
      ),
    );
    if (ok != true) {
      clientId.dispose();
      clientSecret.dispose();
      return;
    }
    try {
      // VendorCreds 六个字段都参与反序列化，OAuth 类字段留空由授权回调填充
      await api.put('/api/v1/access/vendors/$vendor/creds', body: {
        'client_id': clientId.text.trim(),
        'client_secret': clientSecret.text,
        'uid': '',
        'access_token': '',
        'refresh_token': '',
        'expires_at': 0,
      });
      _toast(l10n.commonSuccess);
    } catch (e) {
      _toast('$e');
    } finally {
      clientId.dispose();
      clientSecret.dispose();
    }
  }

  Future<void> _import(String vendor) async {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    try {
      final resp = await api.post('/api/v1/access/vendors/$vendor/import');
      final imported = resp is Map ? resp['imported'] : null;
      final count = resp is Map
          ? (resp['count'] ?? (imported is List ? imported.length : 0))
          : 0;
      final degraded = resp is Map && resp['degraded'] == true;
      _toast('${l10n.adminVendorImported('$count')}'
          '${degraded ? ' · ${l10n.adminVendorDegraded}' : ''}');
    } catch (e) {
      _toast('$e');
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    final api = context.read<ApiClient>();
    final isAdmin = context.watch<AuthController>().isAdmin;
    return Scaffold(
      appBar: AppBar(title: Text(l10n.adminVendorTitle)),
      body: ListView(
        padding: const EdgeInsets.all(12),
        children: [
          for (final v in _vendors)
            Card(
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    ListTile(
                      contentPadding: EdgeInsets.zero,
                      title: Text(v.name),
                      subtitle: Text('${v.key} · ${v.kind}'),
                    ),
                    if (v.key == 'tuya')
                      Padding(
                        padding: const EdgeInsets.only(bottom: 8),
                        child: SelectableText(
                          '${l10n.adminVendorWebhook}: '
                          '${api.baseUrl}/api/v1/access/webhook/tuya',
                          style: Theme.of(context).textTheme.bodySmall,
                        ),
                      ),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        // AK/SK 厂商（huawei/aws/azure）没有 OAuth 授权流程
                        if (v.kind == 'OAuth')
                          FilledButton.tonalIcon(
                            onPressed: () => _authorize(v.key),
                            icon: const Icon(Icons.login),
                            label: Text(l10n.adminVendorOAuth),
                          ),
                        // 凭据写入在网关是 admin-only
                        if (isAdmin)
                          OutlinedButton.icon(
                            onPressed: () => _saveCreds(v.key),
                            icon: const Icon(Icons.key_outlined),
                            label: Text(l10n.adminVendorSaveCreds),
                          ),
                        OutlinedButton.icon(
                          onPressed: () => _import(v.key),
                          icon: const Icon(Icons.download_outlined),
                          label: Text(l10n.adminVendorImport),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}
