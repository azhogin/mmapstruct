class JsonTreeViewer {
    constructor(container, options = {}) {
        this.container = container;
        this.collapseDepth = options.collapseDepth || 3;
    }

    render(data) {
        this.container.innerHTML = this._renderNode(data, 0, '');
        this._attachEvents();
    }

    _renderNode(node, depth, path) {
        if (node === null) return '<span class="jv-null">null</span>';
        if (node === undefined) return '<span class="jv-undefined">undefined</span>';

        const type = Array.isArray(node) ? 'array' : typeof node;

        switch (type) {
            case 'object':
                return this._renderObject(node, depth, path);
            case 'array':
                return this._renderArray(node, depth, path);
            case 'string':
                return this._renderString(node);
            case 'number':
                return `<span class="jv-number">${node}</span>`;
            case 'boolean':
                return `<span class="jv-boolean">${node}</span>`;
            default:
                return `<span>${String(node)}</span>`;
        }
    }

    _renderObject(obj, depth, path) {
        const keys = Object.keys(obj);
        if (keys.length === 0) return '<span class="jv-empty">{}</span>';

        const collapsed = depth >= this.collapseDepth;
        const toggleIcon = collapsed ? '▶' : '▼';
        const displayClass = collapsed ? 'jv-collapsed' : 'jv-expanded';

        let html = `<span class="jv-toggle" data-path="${path}">${toggleIcon}</span>`;
        html += `<span class="jv-bracket">{</span>`;
        html += `<span class="jv-count">${keys.length} field${keys.length !== 1 ? 's' : ''}</span>`;
        html += `<div class="${displayClass}" data-parent="${path}">`;

        for (const key of keys) {
            const childPath = path ? `${path}.${key}` : key;
            html += `<div class="jv-row">`;
            html += `<span class="jv-key" data-path="${childPath}">"${key}":</span>`;
            html += this._renderNode(obj[key], depth + 1, childPath);
            html += `</div>`;
        }

        html += `</div>`;
        html += `<span class="jv-bracket">}</span>`;
        return html;
    }

    _renderArray(arr, depth, path) {
        if (arr.length === 0) return '<span class="jv-empty">[]</span>';

        const collapsed = depth >= this.collapseDepth;
        const toggleIcon = collapsed ? '▶' : '▼';
        const displayClass = collapsed ? 'jv-collapsed' : 'jv-expanded';

        let html = `<span class="jv-toggle" data-path="${path}">${toggleIcon}</span>`;
        html += `<span class="jv-bracket">[</span>`;
        html += `<span class="jv-count">${arr.length} item${arr.length !== 1 ? 's' : ''}</span>`;
        html += `<div class="${displayClass}" data-parent="${path}">`;

        arr.forEach((item, i) => {
            const childPath = `${path}[${i}]`;
            html += `<div class="jv-row jv-array-item">`;
            html += `<span class="jv-index">${i}:</span>`;
            html += this._renderNode(item, depth + 1, childPath);
            html += `</div>`;
        });

        html += `</div>`;
        html += `<span class="jv-bracket">]</span>`;
        return html;
    }

    _renderString(str) {
        const escaped = this._escapeHtml(str);
        const truncated = str.length > 100
            ? escaped.substring(0, 100) + '…'
            : escaped;
        const title = str.length > 100 ? `title="${escaped}"` : '';
        return `<span class="jv-string" ${title}>"${truncated}"</span>`;
    }

    _escapeHtml(str) {
        const div = document.createElement('div');
        div.textContent = str;
        return div.innerHTML;
    }

    _attachEvents() {
        this.container.querySelectorAll('.jv-toggle').forEach(el => {
            el.addEventListener('click', (e) => {
                const path = e.target.dataset.path;
                const child = this.container.querySelector(
                    `[data-parent="${CSS.escape(path)}"]`
                );
                if (child) {
                    const isCollapsed = child.classList.contains('jv-collapsed');
                    child.classList.toggle('jv-collapsed', !isCollapsed);
                    child.classList.toggle('jv-expanded', isCollapsed);
                    e.target.textContent = isCollapsed ? '▼' : '▶';
                }
            });
        });

        this.container.querySelectorAll('.jv-key').forEach(el => {
            el.addEventListener('click', (e) => {
                const path = e.target.dataset.path;
                navigator.clipboard.writeText(path);
                this._showToast(`Copied: ${path}`);
            });
        });
    }

    _showToast(msg) {
        const toast = document.createElement('div');
        toast.className = 'jv-toast';
        toast.textContent = msg;
        document.body.appendChild(toast);
        setTimeout(() => toast.remove(), 2000);
    }
}
