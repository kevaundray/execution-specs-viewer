// Toggle directory visibility in file tree
document.addEventListener('DOMContentLoaded', function() {
    document.querySelectorAll('.dir-toggle').forEach(function(toggle) {
        toggle.addEventListener('click', function() {
            this.classList.toggle('open');
            var children = this.nextElementSibling;
            if (children) {
                children.classList.toggle('collapsed');
            }
        });
    });

    // Expand directories that contain the active file
    var active = document.querySelector('.file-tree a.active');
    if (active) {
        var parent = active.parentElement;
        while (parent) {
            if (parent.classList && parent.classList.contains('dir-children')) {
                parent.classList.remove('collapsed');
                var toggle = parent.previousElementSibling;
                if (toggle && toggle.classList.contains('dir-toggle')) {
                    toggle.classList.add('open');
                }
            }
            parent = parent.parentElement;
        }
    }
});

// Definition card toggle
function toggleCard(header) {
    var body = header.nextElementSibling;
    if (body && body.classList.contains('def-body')) {
        body.classList.toggle('collapsed');
    }
}

function expandAllCards() {
    document.querySelectorAll('.def-body.collapsed').forEach(function(body) {
        body.classList.remove('collapsed');
    });
}

function collapseAllCards() {
    document.querySelectorAll('.def-body').forEach(function(body) {
        body.classList.add('collapsed');
    });
}

// Spec-view card toggle
function toggleSpecCard(header) {
    var body = header.nextElementSibling;
    if (body && body.classList.contains('spec-card-body')) {
        body.classList.toggle('collapsed');
    }
}

function expandAllSpecCards() {
    document.querySelectorAll('.spec-card-body.collapsed').forEach(function(body) {
        body.classList.remove('collapsed');
    });
}

function collapseAllSpecCards() {
    document.querySelectorAll('.spec-card-body').forEach(function(body) {
        body.classList.add('collapsed');
    });
}
