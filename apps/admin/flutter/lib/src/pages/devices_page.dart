import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import 'device_detail_sheet.dart';

/// 设备管理：筛选/分页 + 生命周期操作 + 详情（物模型、指令下发、告警、5s 轮询）。
class DevicesPage extends StatefulWidget {
  const DevicesPage({super.key});

  @override
  State<DevicesPage> createState() => _DevicesPageState();
}

class _DevicesPageState extends State<DevicesPage> {
  static const _pageSize = 20;
  static const _vendors = ['tuya', 'miot', 'huawei', 'aws', 'azure', 'direct'];
  static const _statuses = [
    'online',
    'offline',
    'enabled',
    'disabled',
    'unbound'
  ];

  final _query = TextEditingController();
  String _filter = '';
  String _vendor = '';
  String _status = '';
  String _group = '';
  int _page = 0;
  int _version = 0;
  List<Map<String, dynamic>> _groups = const [];

  @override
  void initState() {
    super.initState();
    _loadGroups();
  }

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  /// 分组筛选是增强项：接口不可用时保持空列表，不影响设备列表。
  Future<void> _loadGroups() async {
    try {
      final api = context.read<ApiClient>();
      final resp = await api.get('/api/devices/groups');
      final groups = resp is Map ? resp['groups'] : resp;
      if (groups is List && mounted) {
        setState(() => _groups =
            groups.whereType<Map<String, dynamic>>().toList());
      }
    } catch (_) {}
  }

  Future<List<Device>> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/devices',
        query: _group.isEmpty ? null : {'group_id': _group});
    final list = resp is Map ? resp['devices'] : resp;
    return parseList<Device>(list, Device.fromJson);
  }

  void _refilter() => setState(() {
        _page = 0;
        _version++;
      });

  Future<void> _lifecycle(Device d, String action) async {
    final api = context.read<ApiClient>();
    final l10n = AppLocalizations.of(context)!;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(action == 'delete' ? l10n.commonDelete : l10n.deviceUnbind),
        content: Text(l10n.deviceLifecycleConfirm),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: Text(l10n.commonCancel)),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: Text(l10n.commonConfirm)),
        ],
      ),
    );
    if (confirmed != true) return;
    try {
      switch (action) {
        case 'enable':
          await api.put('/api/devices/${d.id}', body: {'status': 'enabled'});
        case 'disable':
          await api.put('/api/devices/${d.id}', body: {'status': 'disabled'});
        case 'unbind':
          await api.post('/api/devices/${d.id}/unbind');
        case 'delete':
          await api.delete('/api/devices/${d.id}');
      }
      _showSnack(l10n.commonSuccess);
      _refilter();
    } catch (e) {
      _showSnack('$e');
    }
  }

  void _showSnack(String msg) {
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _showDetail(Device d) async {
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: (ctx) => FractionallySizedBox(
        heightFactor: 0.9,
        child: DeviceDetailSheet(device: d),
      ),
    );
    // 详情里可能下发了指令/改了状态，关闭后刷新列表
    if (mounted) _refilter();
  }

  Widget _filterRow() {
    final l10n = AppLocalizations.of(context)!;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
      child: Wrap(
        spacing: 8,
        runSpacing: 8,
        children: [
          SizedBox(
            width: 220,
            child: TextField(
              controller: _query,
              onChanged: (v) => setState(() {
                _filter = v.trim();
                _page = 0;
              }),
              decoration: InputDecoration(
                hintText: l10n.commonSearch,
                prefixIcon: const Icon(Icons.search),
                border: const OutlineInputBorder(),
                isDense: true,
              ),
            ),
          ),
          _filterDropdown(
            l10n.adminFilterVendor,
            _vendor,
            const ['', ..._vendors],
            (v) {
              _vendor = v;
              _refilter();
            },
          ),
          _filterDropdown(
            l10n.adminFilterStatus,
            _status,
            const ['', ..._statuses],
            (v) {
              _status = v;
              _refilter();
            },
          ),
          _filterDropdown(
            l10n.adminFilterGroup,
            _group,
            ['', ..._groups.map((g) => '${g['id'] ?? ''}')],
            (v) {
              _group = v;
              _refilter();
            },
            labelOf: (id) {
              for (final g in _groups) {
                if ('${g['id']}' == id) return '${g['name'] ?? id}';
              }
              return id;
            },
          ),
        ],
      ),
    );
  }

  Widget _filterDropdown(String label, String value, List<String> values,
      ValueChanged<String> onChanged,
      {String Function(String)? labelOf}) {
    final l10n = AppLocalizations.of(context)!;
    // 选中项可能随分组列表刷新消失，保留在 options 里避免 Dropdown 断言失败
    final options = values.contains(value) ? values : [...values, value];
    return SizedBox(
      width: 150,
      child: DropdownButtonFormField<String>(
        initialValue: value.isEmpty ? values.first : value,
        isDense: true,
        decoration: InputDecoration(
          labelText: label,
          border: const OutlineInputBorder(),
          isDense: true,
        ),
        items: [
          for (final v in options)
            DropdownMenuItem(
              value: v,
              child: Text(v.isEmpty
                  ? l10n.adminFilterAll
                  : (labelOf?.call(v) ?? v)),
            ),
        ],
        onChanged: (v) => onChanged(v ?? ''),
      ),
    );
  }

  Widget _list(BuildContext context, List<Device> devices) {
    final l10n = AppLocalizations.of(context)!;
    final filtered = devices.where((d) {
      if (_filter.isNotEmpty &&
          !d.name.contains(_filter) &&
          !d.id.contains(_filter) &&
          !d.vendor.contains(_filter)) {
        return false;
      }
      if (_vendor.isNotEmpty && d.vendor != _vendor) return false;
      if (_status.isNotEmpty && d.status != _status) return false;
      return true;
    }).toList();
    if (filtered.isEmpty) {
      return Center(child: Text(l10n.commonEmpty));
    }
    final pages = (filtered.length + _pageSize - 1) ~/ _pageSize;
    final page = _page.clamp(0, pages - 1);
    final slice =
        filtered.skip(page * _pageSize).take(_pageSize).toList();
    return ListView.builder(
      itemCount: slice.length + (pages > 1 ? 1 : 0),
      itemBuilder: (context, i) {
        if (i >= slice.length) {
          return Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                IconButton(
                  icon: const Icon(Icons.chevron_left),
                  onPressed:
                      page > 0 ? () => setState(() => _page = page - 1) : null,
                ),
                Text('${page + 1} / $pages'),
                IconButton(
                  icon: const Icon(Icons.chevron_right),
                  onPressed: page < pages - 1
                      ? () => setState(() => _page = page + 1)
                      : null,
                ),
              ],
            ),
          );
        }
        final d = slice[i];
        return ListTile(
          leading: Icon(
            d.online ? Icons.circle : Icons.circle_outlined,
            color: d.online ? Colors.green : Colors.grey,
          ),
          title: Text(d.name.isEmpty ? d.id : d.name),
          subtitle: Text('${d.vendor} · ${d.id}'),
          onTap: () => _showDetail(d),
          trailing: PopupMenuButton<String>(
            onSelected: (a) => _lifecycle(d, a),
            itemBuilder: (ctx) => [
              PopupMenuItem(value: 'enable', child: Text(l10n.deviceEnable)),
              PopupMenuItem(value: 'disable', child: Text(l10n.deviceDisable)),
              PopupMenuItem(value: 'unbind', child: Text(l10n.deviceUnbind)),
              PopupMenuItem(value: 'delete', child: Text(l10n.commonDelete)),
            ],
          ),
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return Scaffold(
      appBar: AppBar(title: Text(l10n.navDevices)),
      body: Column(
        children: [
          _filterRow(),
          Expanded(
            child: ApiList<Device>(
              key: ValueKey('$_version|$_group'),
              load: _load,
              emptyText: l10n.commonEmpty,
              builder: _list,
            ),
          ),
        ],
      ),
    );
  }
}
